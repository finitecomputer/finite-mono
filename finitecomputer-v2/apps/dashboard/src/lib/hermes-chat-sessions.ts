import type {
  HostedChatMessage,
  HostedChatSummary,
  HostedChatTopic,
} from "@/lib/hosted-web-device";
import { HOME_TOPIC_ID } from "@/lib/hosted-web-chat-topics";

/**
 * Pure mapping from native Hermes session rows (`GET api/sessions`) and
 * message rows (`GET api/sessions/{id}/messages`) onto the shared chat view.
 *
 * Hermes is the single store here. Sessions a messaging platform routed in
 * (they carry a `chat_id`, e.g. Finite Chat rooms) are grouped per platform
 * room and are read-only in this view: a reply sent from the browser would
 * land in Hermes but never reach that platform. Native sessions (no
 * `chat_id`) live in the Home topic and accept new turns.
 */

export const HERMES_ROOM_ID = "hermes";
export const HERMES_AGENT_ACCOUNT = "hermes-agent";
export const HERMES_VIEWER_ACCOUNT = "me";

export type HermesSessionRow = {
  id: string;
  source: string;
  title: string | null;
  preview: string;
  chatId: string | null;
  chatType: string | null;
  threadId: string | null;
  displayName: string | null;
  startedAt: number;
  lastActive: number;
  messageCount: number;
  archived: boolean;
};

export type HermesSessionPage = {
  sessions: HermesSessionRow[];
  total: number;
};

export type HermesMessageRow = {
  id: number | null;
  role: string;
  content: string;
  toolName: string | null;
  toolCalls: { name: string; arguments: string }[];
  reasoning: string | null;
  timestamp: number;
  hidden: boolean;
  /** A compaction summary projected for display; the originals are listed too. */
  summary: boolean;
};

export type HermesTopicGroup = {
  topic: HostedChatTopic;
  readOnly: boolean;
};

export function parseHermesSessionPage(value: unknown): HermesSessionPage {
  const page = record(value);
  if (!Array.isArray(page.sessions)) {
    throw new Error("Hermes returned an unexpected session list.");
  }
  const sessions: HermesSessionRow[] = [];
  for (const raw of page.sessions) {
    const row = record(raw);
    if (typeof row.id !== "string" || !row.id) continue;
    sessions.push({
      id: row.id,
      source: stringOr(row.source, "unknown"),
      title: optionalString(row.title),
      preview: stringOr(row.preview, ""),
      chatId: optionalString(row.chat_id),
      chatType: optionalString(row.chat_type),
      threadId: optionalString(row.thread_id),
      displayName: optionalString(row.display_name),
      startedAt: numberOr(row.started_at, 0),
      lastActive: numberOr(row.last_active, numberOr(row.started_at, 0)),
      messageCount: numberOr(row.message_count, 0),
      archived: Boolean(row.archived),
    });
  }
  return { sessions, total: numberOr(page.total, sessions.length) };
}

export function parseHermesMessages(value: unknown): HermesMessageRow[] {
  const body = record(value);
  if (!Array.isArray(body.messages)) {
    throw new Error("Hermes returned an unexpected transcript.");
  }
  return body.messages.map((raw) => {
    const row = record(raw);
    const display = optionalString(row.display_content);
    return {
      id: typeof row.id === "number" ? row.id : null,
      role: stringOr(row.role, "unknown"),
      content: display ?? contentText(row.content),
      toolName: optionalString(row.tool_name),
      toolCalls: toolCalls(row.tool_calls),
      reasoning: optionalString(row.reasoning) ?? optionalString(row.reasoning_content),
      timestamp: numberOr(row.timestamp, 0),
      hidden: row.display_kind === "hidden" || row.display_kind === "internal_notification",
      summary: display !== null,
    };
  });
}

/** Sources whose sessions belong to another app or to the agent itself. */
const NON_WEB_SOURCES = new Set(["finitechat", "simplex", "cron", "kanban", "tool"]);

/**
 * Platform-routed sessions (and chats carried over from Finite Chat, which
 * Hermes imports without routing) are read-only in the browser view, as are
 * scheduled runs. Everything else is a native chat.
 */
export function hermesSessionReadOnly(session: Pick<HermesSessionRow, "chatId" | "source">) {
  return Boolean(session.chatId) || NON_WEB_SOURCES.has(session.source);
}

export function hermesTopicIdForSession(session: HermesSessionRow) {
  if (session.chatId) return `platform:${session.source}:${session.chatId}`;
  return hermesSessionReadOnly(session) ? `platform:${session.source}` : HOME_TOPIC_ID;
}

export function hermesChatSummary(session: HermesSessionRow): HostedChatSummary {
  return {
    chat_id: session.id,
    title: session.title || firstLine(session.preview) || "Untitled chat",
    last_message_preview: session.preview,
    unread_count: 0,
    message_count: session.messageCount,
    started_seq: Math.floor(session.startedAt),
    updated_seq: Math.floor(session.lastActive),
    active: false,
    archived: session.archived,
  };
}

/**
 * Group sessions into topics: one Home topic for native chats (always
 * present so New chat has a home) and one read-only topic per platform room.
 * `extraHomeChats` are local drafts Hermes does not list until their first
 * prompt.
 */
export function hermesTopicGroups(
  sessions: HermesSessionRow[],
  extraHomeChats: HostedChatSummary[] = [],
): HermesTopicGroup[] {
  const home: HostedChatSummary[] = [...extraHomeChats];
  const platforms = new Map<string, { sessions: HermesSessionRow[] }>();
  for (const session of sessions) {
    const topicId = hermesTopicIdForSession(session);
    if (topicId === HOME_TOPIC_ID) {
      home.push(hermesChatSummary(session));
      continue;
    }
    const group = platforms.get(topicId) ?? { sessions: [] };
    group.sessions.push(session);
    platforms.set(topicId, group);
  }
  const groups: HermesTopicGroup[] = [{
    readOnly: false,
    topic: topicFrom(HOME_TOPIC_ID, "Hermes", "Chats started from the web", home),
  }];
  for (const [topicId, group] of platforms) {
    const first = group.sessions[0]!;
    const chats = group.sessions.map(hermesChatSummary);
    const label = first.chatId
      ? `${platformLabel(first.source)}${first.displayName ? ` · ${first.displayName}` : ""}`
      : first.source === "finitechat"
        ? "Finite Chat · earlier"
        : platformLabel(first.source);
    groups.push({
      readOnly: true,
      topic: topicFrom(topicId, label, "Read-only history", chats),
    });
  }
  return groups;
}

/**
 * Map stored Hermes rows onto transcript rows. User and assistant prose stay
 * prose; reasoning, tool calls and tool results ride `kind: "tool"` rows so
 * the shared transcript collapses them into its tool rollup.
 */
export function hermesTranscript(
  rows: HermesMessageRow[],
  chatId: string,
  topicId: string,
  agentName: string,
): HostedChatMessage[] {
  const mapped: HostedChatMessage[] = [];
  rows.forEach((row, index) => {
    if (row.hidden) return;
    const key = row.id ?? index;
    const at = row.timestamp;
    if (row.summary) {
      if (row.content) mapped.push(toolRow(`Context summary: ${truncate(row.content, 2_000)}`, `${chatId}:${key}:summary`, chatId, topicId, at, agentName));
      return;
    }
    if (row.role === "user") {
      if (row.content) mapped.push(hermesMessage({ role: "user", text: row.content, chatId, topicId, id: `${chatId}:${key}`, at, agentName }));
      return;
    }
    if (row.role === "assistant") {
      if (row.reasoning) {
        mapped.push(toolRow(row.reasoning, `${chatId}:${key}:think`, chatId, topicId, at, agentName));
      }
      row.toolCalls.forEach((call, callIndex) => {
        mapped.push(toolRow(
          call.arguments ? `${call.name}: ${truncate(call.arguments, 400)}` : call.name,
          `${chatId}:${key}:call${callIndex}`, chatId, topicId, at, agentName,
        ));
      });
      if (row.content) {
        mapped.push(hermesMessage({ role: "assistant", text: row.content, chatId, topicId, id: `${chatId}:${key}`, at, agentName }));
      }
      return;
    }
    if (row.role === "tool") {
      const label = [row.toolName, truncate(row.content, 2_000)].filter(Boolean).join(": ");
      if (label) mapped.push(toolRow(label, `${chatId}:${key}:tool`, chatId, topicId, at, agentName));
    }
  });
  return mapped;
}

export function hermesMessage({
  role,
  text,
  chatId,
  topicId,
  id,
  at,
  agentName,
  status = "complete",
}: {
  role: "user" | "assistant";
  text: string;
  chatId: string;
  topicId: string;
  id: string;
  at: number;
  agentName: string;
  status?: "running" | "complete";
}): HostedChatMessage {
  const mine = role === "user";
  const seconds = Math.floor(at || Date.now() / 1000);
  return {
    room_id: HERMES_ROOM_ID,
    seq: 0,
    message_id: id,
    conversation_id: topicId,
    chat_id: chatId,
    sender_account_id: mine ? HERMES_VIEWER_ACCOUNT : HERMES_AGENT_ACCOUNT,
    sender_device_id: mine ? "web" : "hermes",
    sender_display_name: mine ? "You" : agentName,
    sender_npub: null,
    text,
    display_content: text,
    reply_to_message_id: null,
    is_mine: mine,
    outbound_delivery: mine ? { local_send: "Sent", server_delivery: "Delivered" } : null,
    media: [],
    kind: "message",
    status,
    final_delivery: status === "complete",
    edit_of_message_id: null,
    timestamp_unix_seconds: seconds,
    display_timestamp: new Date(seconds * 1000).toLocaleTimeString([], {
      hour: "numeric",
      minute: "2-digit",
    }),
  };
}

function toolRow(
  text: string,
  id: string,
  chatId: string,
  topicId: string,
  at: number,
  agentName: string,
  status: "running" | "complete" = "complete",
): HostedChatMessage {
  return {
    ...hermesMessage({ role: "assistant", text, chatId, topicId, id, at, agentName, status }),
    kind: "tool",
  };
}

export function hermesToolRow(
  text: string,
  id: string,
  chatId: string,
  topicId: string,
  agentName: string,
  status: "running" | "complete",
) {
  return toolRow(text, id, chatId, topicId, Date.now() / 1000, agentName, status);
}

function topicFrom(
  topicId: string,
  title: string,
  description: string,
  chats: HostedChatSummary[],
): HostedChatTopic {
  const ordered = [...chats].sort((left, right) => right.updated_seq - left.updated_seq);
  return {
    room_id: HERMES_ROOM_ID,
    topic_id: topicId,
    title,
    description,
    last_message_preview: ordered[0]?.last_message_preview ?? "",
    unread_count: 0,
    message_count: ordered.length,
    created_seq: 0,
    updated_seq: ordered[0]?.updated_seq ?? 0,
    archived: false,
    active_chat_id: null,
    chats: ordered,
  };
}

function platformLabel(source: string) {
  if (source === "finitechat") return "Finite Chat";
  if (source === "simplex") return "SimpleX";
  if (source === "cron") return "Scheduled tasks";
  return source.charAt(0).toUpperCase() + source.slice(1);
}

function contentText(value: unknown): string {
  if (typeof value === "string") return value;
  if (Array.isArray(value)) {
    return value
      .map((part) => {
        const item = record(part);
        return typeof item.text === "string" ? item.text : "";
      })
      .filter(Boolean)
      .join("\n");
  }
  return "";
}

function toolCalls(value: unknown): { name: string; arguments: string }[] {
  let parsed = value;
  if (typeof value === "string") {
    try {
      parsed = JSON.parse(value);
    } catch {
      return [];
    }
  }
  if (!Array.isArray(parsed)) return [];
  return parsed.flatMap((raw) => {
    const call = record(raw);
    const fn = record(call.function);
    const name = optionalString(fn.name) ?? optionalString(call.name);
    if (!name) return [];
    const args = fn.arguments ?? call.arguments;
    return [{ name, arguments: typeof args === "string" ? args : args ? JSON.stringify(args) : "" }];
  });
}

function firstLine(text: string) {
  return text.split("\n", 1)[0]?.trim().slice(0, 80) ?? "";
}

function truncate(text: string, max: number) {
  return text.length > max ? `${text.slice(0, max)}…` : text;
}

function record(value: unknown): Record<string, unknown> {
  return value && typeof value === "object" ? value as Record<string, unknown> : {};
}

function optionalString(value: unknown): string | null {
  return typeof value === "string" && value.trim() ? value : null;
}

function stringOr(value: unknown, fallback: string) {
  return typeof value === "string" ? value : fallback;
}

function numberOr(value: unknown, fallback: number) {
  return typeof value === "number" && Number.isFinite(value) ? value : fallback;
}
