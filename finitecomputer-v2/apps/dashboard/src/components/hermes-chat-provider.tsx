"use client";

/**
 * Spike: the hermes-agent tui_gateway backend for the EXISTING chat UI.
 *
 * Implements the same context the finitechat hosted-device provider does
 * (HostedChatContextValue — see hosted-chat-provider.tsx) so AgentSidebar,
 * HostedWebChat, and every chat component render UNCHANGED over hermes
 * sessions. No new UI lives here; this file is transport only.
 *
 * Mapping (all verified against hermes v2026.8.3, see NOTES.md):
 *   room          → the single gateway ("hermes")
 *   topics        → projects.tree projects (by session cwd) + a flat
 *                   "Recents" topic for sessions no project claims
 *   chats         → gateway sessions (plus local unprompted drafts, which
 *                   hermes keeps invisible until their first prompt)
 *   messages      → session.resume transcript + streamed
 *                   reasoning/message deltas as one "running" assistant
 *                   message finalized by message.complete
 *   SendMessage   → prompt.submit
 *   OpenChat      → session.resume (returns the transcript inline)
 *   CreateTopic   → projects.create; StartTopicChatIntent → session.create
 *                   with cwd = the project's path (topics ARE cwds)
 *   RenameChat    → session.title; MarkRoomRead/SetTyping → quiet no-ops
 *   archive, profiles, devices → honest gaps (no gateway analogue yet)
 *
 * Connection info is inlined from public env:
 *   NEXT_PUBLIC_HERMES_GATEWAY_WS_URL / NEXT_PUBLIC_HERMES_GATEWAY_TOKEN
 */

import type { ReactNode } from "react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import {
  HostedChatContext,
  type HostedChatContextValue,
} from "@/components/hosted-chat-provider";
import type {
  HostedChatAction,
  HostedChatMessage,
  HostedChatState,
  HostedChatSummary,
  HostedChatTopic,
} from "@/lib/hosted-web-device";
import type { HostedChatRetryAttempt } from "@/lib/hosted-web-chat-retry";
import type { PendingChatRefreshTarget } from "@/lib/hosted-web-chat-refresh";

const GATEWAY_WS_URL =
  process.env.NEXT_PUBLIC_HERMES_GATEWAY_WS_URL || "ws://127.0.0.1:9120/api/ws";
const GATEWAY_TOKEN = process.env.NEXT_PUBLIC_HERMES_GATEWAY_TOKEN || "";

const ROOM_ID = "hermes";
const RECENTS_TOPIC_ID = "recents";
const MY_ACCOUNT = "me";
const AGENT_ACCOUNT = "hermes-agent";
const AGENT_NAME = "Hermes";

type GatewayInbound = {
  jsonrpc: "2.0";
  id?: number;
  method?: string;
  params?: GatewayEvent;
  result?: unknown;
  error?: { code?: number; message?: string };
};

type GatewayEvent = {
  type?: string;
  session_id?: string;
  payload?: Record<string, unknown>;
};

type GatewaySession = {
  id: string;
  title: string;
  preview: string;
  started_at: number;
  message_count: number;
  source: string;
};

type GatewayProject = {
  id: string;
  label: string;
  path: string;
  sessionCount: number;
  previewSessions: GatewaySession[];
};

type GatewayMessage = { role: string; text: string };

/** A chat the UI knows about: a persisted gateway session or a local draft. */
type ChatEntry = {
  summary: HostedChatSummary;
  /** Short gateway handle for RPCs; drafts keep their session.create id. */
  handleId: string;
  /** Project id whose cwd spawned it (drives StartTopicChatIntent). */
  topicId: string;
  /** Present while a turn is streaming onto this chat. */
  streaming: { reasoning: string; answer: string } | null;
};

type GatewayCall = (method: string, params?: Record<string, unknown>) => Promise<unknown>;

export function HermesChatProvider({ children }: { children: ReactNode }) {
  const [state, setState] = useState<HostedChatState | null>(null);
  const [transportError, setTransportError] = useState<string | null>(null);
  const [streamConnected, setStreamConnected] = useState(false);

  const revRef = useRef(1);
  const seqRef = useRef(1);
  const chatsRef = useRef(new Map<string, ChatEntry>()); // keyed by chat_id
  const transcriptRef = useRef(new Map<string, HostedChatMessage[]>()); // chat_id → messages
  const projectsRef = useRef<GatewayProject[]>([]);
  const scopedRef = useRef<Set<string>>(new Set());
  const sessionsRef = useRef<GatewaySession[]>([]);
  const selectedRef = useRef<{ topicId: string | null; chatId: string | null }>({
    topicId: null,
    chatId: null,
  });
  const typingRef = useRef(false);
  const socketRef = useRef<WebSocket | null>(null);
  const nextIdRef = useRef(1);
  const pendingRef = useRef(
    new Map<number, { resolve: (value: unknown) => void; reject: (reason: Error) => void }>()
  );
  const callRef = useRef<GatewayCall | null>(null);

  const stateRef = useRef<HostedChatState | null>(null);

  const publish = useCallback(() => {
    const topics = topicsFrom(
      projectsRef.current,
      sessionsRef.current,
      scopedRef.current,
      chatsRef.current
    );
    const selected = selectedRef.current;
    const next: HostedChatState = {
      rev: revRef.current++,
      identity: { account_id: MY_ACCOUNT, device_id: "web-gateway" },
      rooms: [
        {
          room_id: ROOM_ID,
          display_name: AGENT_NAME,
          state: "Connected",
          status: "ok",
          user_status_text: "",
          last_message_preview: sessionsRef.current[0]?.preview ?? "",
          unread_count: 0,
          can_load_older: false,
          is_agent_chat: true,
        },
      ],
      topics,
      selected_room_id: ROOM_ID,
      selected_topic_id: selected.topicId,
      selected_chat_id: selected.chatId,
      active_profile_id: null,
      status: "ok",
      toast: null,
      messages: (selected.chatId ? transcriptRef.current.get(selected.chatId) : null) ?? [],
      profiles: [
        {
          account_id: AGENT_ACCOUNT,
          npub: "",
          display_name: AGENT_NAME,
          about: null,
          picture: null,
          stale: false,
          is_agent: true,
        },
      ],
      devices: [],
      typing_members: typingRef.current
        ? [
            {
              room_id: ROOM_ID,
              topic_id: selected.topicId,
              chat_id: selected.chatId,
              account_id: AGENT_ACCOUNT,
              device_id: "gateway",
              display_name: AGENT_NAME,
              activity_kind: "thinking",
            },
          ]
        : [],
      hosted_agent_binding: {
        version: 1,
        project_id: "hermes-gateway",
        human_account_id: MY_ACCOUNT,
        agent_account_id: AGENT_ACCOUNT,
        agent_npub: "",
        canonical_room_id: ROOM_ID,
        associated_room_ids: [ROOM_ID],
      },
      flow: {
        notice_text: null,
        notice_busy: false,
        scan_in_flight: false,
        scan_result: "",
        image_upload_url: null,
      },
    };
    stateRef.current = next;
    setState(next);
  }, []);

  const refreshLists = useCallback(async () => {
    const call = callRef.current;
    if (!call) return;
    const [treeResult, listResult] = await Promise.all([
      call("projects.tree", { preview_limit: 50 }),
      call("session.list", { limit: 100 }),
    ]);
    const tree = treeResult as { projects?: GatewayProject[]; scoped_session_ids?: string[] } | null;
    const list = listResult as { sessions?: GatewaySession[] } | null;
    projectsRef.current = (tree?.projects ?? []).filter((project) => project.sessionCount > 0);
    scopedRef.current = new Set(tree?.scoped_session_ids ?? []);
    const sessions = list?.sessions ?? [];
    sessionsRef.current = sessions;
    // Adopt persisted sessions; drop local drafts that have materialized.
    for (const session of sessions) {
      const existing = chatsRef.current.get(session.id);
      chatsRef.current.set(session.id, {
        summary: summaryFromSession(session),
        handleId: existing?.handleId ?? "",
        topicId: existing?.topicId ?? topicIdForSession(session.id, scopedRef.current),
        streaming: existing?.streaming ?? null,
      });
    }
    for (const [chatId, entry] of chatsRef.current) {
      if (chatId.startsWith("draft:") && !entry.streaming) {
        // Drafts stay until their first prompt materializes a stored row.
        continue;
      }
    }
    publish();
  }, [publish]);

  const handleEvent = useCallback(
    (event: GatewayEvent) => {
      const type = event.type ?? "";
      if (type === "sessions.changed") {
        void refreshLists().catch(() => undefined);
        return;
      }
      const chat = [...chatsRef.current.values()].find(
        (candidate) => event.session_id && candidate.handleId === event.session_id
      );
      if (!chat) return;
      const chatId = chat.summary.chat_id;
      if (type === "message.start") {
        chat.streaming = { reasoning: "", answer: "" };
        typingRef.current = true;
        publish();
      } else if (type === "reasoning.delta") {
        if (chat.streaming) {
          chat.streaming.reasoning += String(event.payload?.text ?? "");
          replaceStreamingMessage(chatId, chat.streaming);
          publish();
        }
      } else if (type === "message.delta") {
        if (chat.streaming) {
          chat.streaming.answer += String(event.payload?.text ?? "");
          replaceStreamingMessage(chatId, chat.streaming);
          publish();
        }
      } else if (type === "message.complete") {
        const text = String(event.payload?.text ?? "");
        chat.streaming = null;
        typingRef.current = false;
        const messages = transcriptRef.current.get(chatId) ?? [];
        const streamingIndex = messages.findIndex((message) => message.status === "running");
        const finalMessage = gatewayMessage("assistant", text, chatId, false);
        if (streamingIndex >= 0) messages.splice(streamingIndex, 1, finalMessage);
        else messages.push(finalMessage);
        transcriptRef.current.set(chatId, messages);
        publish();
      }
    },
    [publish, refreshLists]
  );

  useEffect(() => {
    let disposed = false;
    let retryMs = 500;
    let retryTimer: ReturnType<typeof setTimeout> | undefined;

    const call: GatewayCall = (method, params = {}) =>
      new Promise<unknown>((resolve, reject) => {
        const socket = socketRef.current;
        if (socket == null || socket.readyState !== WebSocket.OPEN) {
          reject(new Error("hermes gateway is not connected"));
          return;
        }
        const id = nextIdRef.current++;
        pendingRef.current.set(id, { resolve, reject });
        socket.send(JSON.stringify({ jsonrpc: "2.0", id, method, params }));
      });
    callRef.current = call;

    const connect = () => {
      if (disposed) return;
      const target = GATEWAY_TOKEN
        ? `${GATEWAY_WS_URL}?token=${encodeURIComponent(GATEWAY_TOKEN)}`
        : GATEWAY_WS_URL;
      const socket = new WebSocket(target);
      socketRef.current = socket;

      socket.addEventListener("open", () => {
        retryMs = 500;
        setStreamConnected(true);
        setTransportError(null);
        void refreshLists().catch(() => undefined);
      });

      socket.addEventListener("message", (event: MessageEvent) => {
        let message: GatewayInbound;
        try {
          message = JSON.parse(String(event.data));
        } catch {
          return;
        }
        if (message.method === "event" && message.params?.type) {
          handleEvent(message.params);
          return;
        }
        if (message.id == null) return;
        const pending = pendingRef.current.get(message.id);
        if (!pending) return;
        pendingRef.current.delete(message.id);
        if (message.error) {
          pending.reject(
            new Error(`${message.error.message ?? "gateway error"} (${message.error.code ?? "?"})`)
          );
        } else {
          pending.resolve(message.result);
        }
      });

      const reconnect = () => {
        if (disposed) return;
        for (const pending of pendingRef.current.values()) {
          pending.reject(new Error("hermes gateway connection lost"));
        }
        pendingRef.current.clear();
        setStreamConnected(false);
        setTransportError(`gateway unreachable at ${GATEWAY_WS_URL}`);
        retryTimer = setTimeout(connect, retryMs);
        retryMs = Math.min(retryMs * 2, 10_000);
      };
      socket.addEventListener("close", reconnect);
      socket.addEventListener("error", reconnect);
    };

    connect();
    return () => {
      disposed = true;
      if (retryTimer) clearTimeout(retryTimer);
      socketRef.current?.close();
    };
  }, [handleEvent, refreshLists]);

  const dispatch = useCallback(
    async (action: HostedChatAction): Promise<HostedChatState> => {
      const call = callRef.current;
      if (!call) throw new Error("hermes gateway is not connected");

      if ("OpenChat" in action) {
        const { chat_id } = action.OpenChat;
        selectedRef.current = {
          topicId: action.OpenChat.topic_id,
          chatId: chat_id,
        };
        const entry = chatsRef.current.get(chat_id);
        if (entry && !chat_id.startsWith("draft:") && !transcriptRef.current.has(chat_id)) {
          const resumed = (await call("session.resume", { session_id: chat_id })) as {
            session_id: string;
            messages?: GatewayMessage[];
          } | null;
          entry.handleId = resumed?.session_id ?? entry.handleId;
          transcriptRef.current.set(
            chat_id,
            (resumed?.messages ?? []).map((message) =>
              gatewayMessage(message.role, message.text, chat_id, true)
            )
          );
        }
        publish();
        return currentState();
      }

      if ("StartTopicChatIntent" in action) {
        const { topic_id } = action.StartTopicChatIntent;
        const project = projectsRef.current.find(
          (candidate) => topicIdForProject(candidate) === topic_id
        );
        const created = (await call("session.create", {
          source: "gateway",
          cols: 100,
          ...(project?.path ? { cwd: project.path } : {}),
        })) as { session_id: string } | null;
        const draftId = `draft:${created?.session_id ?? crypto.randomUUID()}`;
        chatsRef.current.set(draftId, {
          summary: {
            chat_id: draftId,
            title: "New chat",
            last_message_preview: "",
            unread_count: 0,
            message_count: 0,
            started_seq: 0,
            updated_seq: seqRef.current++,
            active: true,
            archived: false,
          },
          handleId: created?.session_id ?? "",
          topicId: topic_id,
          streaming: null,
        });
        transcriptRef.current.set(draftId, []);
        selectedRef.current = { topicId: topic_id, chatId: draftId };
        publish();
        return currentState();
      }

      if ("CreateTopic" in action) {
        await call("projects.create", { name: action.CreateTopic.title });
        await refreshLists();
        return currentState();
      }

      if ("SendMessage" in action) {
        const { text } = action.SendMessage;
        const chatId = selectedRef.current.chatId;
        const entry = chatId ? chatsRef.current.get(chatId) : null;
        if (!entry || !chatId) throw new Error("no chat selected");
        let handle = entry.handleId;
        if (!handle) {
          const created = (await call("session.create", {
            source: "gateway",
            cols: 100,
          })) as { session_id: string } | null;
          handle = created?.session_id ?? "";
          entry.handleId = handle;
        }
        const messages = transcriptRef.current.get(chatId) ?? [];
        messages.push(gatewayMessage("user", text, chatId, true));
        transcriptRef.current.set(chatId, messages);
        await call("prompt.submit", { session_id: handle, text });
        entry.streaming = { reasoning: "", answer: "" };
        typingRef.current = true;
        // The draft will be replaced by its stored row once it materializes;
        // keep the local entry selected until then so the view does not jump.
        publish();
        return currentState();
      }

      if ("RenameChat" in action) {
        const { chat_id, title } = action.RenameChat;
        const entry = chatsRef.current.get(chat_id);
        if (entry?.handleId) {
          await call("session.title", { session_id: entry.handleId, title });
        } else if (!chat_id.startsWith("draft:")) {
          await call("session.title", { session_id: chat_id, title });
        }
        await refreshLists();
        return currentState();
      }

      // MarkRoomRead, SetTyping, SetChatArchived and the profile/group flows
      // have no gateway analogue yet; they stay quiet no-ops so the shared UI
      // keeps working. Gaps are tracked in NOTES.md.
      publish();
      return currentState();
    },
    [publish, refreshLists]
  );

  const value = useMemo<HostedChatContextValue>(
    () => ({
      apiBase: "hermes-gateway",
      state,
      transportError,
      claimError: null,
      streamConnected,
      ownerClaimed: true,
      bindingRecoveryRequired: false,
      selectionPending: false,
      load: async () => {
        try {
          await refreshLists();
          setTransportError(null);
          return "succeeded" satisfies HostedChatRetryAttempt;
        } catch {
          setTransportError(`gateway unreachable at ${GATEWAY_WS_URL}`);
          return "stop" satisfies HostedChatRetryAttempt;
        }
      },
      claimOwner: async () => "succeeded" as HostedChatRetryAttempt,
      recoverBinding: async () => "succeeded" as HostedChatRetryAttempt,
      dispatch,
      dispatchQuiet: async (action: HostedChatAction) => {
        try {
          return await dispatch(action);
        } catch {
          return null;
        }
      },
      refreshPendingChat: async (_target: PendingChatRefreshTarget) => false,
      uploadAttachments: async () => {
        throw new Error("attachments are not wired to the gateway yet");
      },
      attachmentUrl: () => "#",
    }),
    [dispatch, refreshLists, state, streamConnected, transportError]
  );

  function currentState(): HostedChatState {
    // Dispatch callers only need the freshest published snapshot.
    return (
      stateRef.current ?? {
        rev: 0,
        identity: { account_id: MY_ACCOUNT, device_id: "web-gateway" },
        rooms: [],
        topics: [],
        selected_room_id: null,
        selected_topic_id: null,
        selected_chat_id: null,
        status: "ok",
        messages: [],
        profiles: [],
        devices: [],
        typing_members: [],
        hosted_agent_binding: null,
        flow: { notice_busy: false, scan_in_flight: false, scan_result: "" },
      }
    );
  }

  function replaceStreamingMessage(chatId: string, streaming: { reasoning: string; answer: string }) {
    const messages = transcriptRef.current.get(chatId) ?? [];
    const text = streaming.answer || streaming.reasoning || "…";
    const running = gatewayMessage("assistant", text, chatId, false, "running");
    const index = messages.findIndex((message) => message.status === "running");
    if (index >= 0) messages.splice(index, 1, running);
    else messages.push(running);
    transcriptRef.current.set(chatId, messages);
  }

  return <HostedChatContext.Provider value={value}>{children}</HostedChatContext.Provider>;
}

function summaryFromSession(session: GatewaySession): HostedChatSummary {
  return {
    chat_id: session.id,
    title: session.title || session.preview || "Untitled session",
    last_message_preview: session.preview,
    unread_count: 0,
    message_count: session.message_count,
    started_seq: 0,
    updated_seq: Math.floor(session.started_at),
    active: false,
    archived: false,
  };
}

function topicIdForProject(project: GatewayProject): string {
  return `project:${project.id}`;
}

function topicIdForSession(sessionId: string, scoped: Set<string>): string {
  return scoped.has(sessionId) ? "" : RECENTS_TOPIC_ID;
}

function topicsFrom(
  projects: GatewayProject[],
  sessions: GatewaySession[],
  scoped: Set<string>,
  chats: Map<string, ChatEntry>
): HostedChatTopic[] {
  const topics: HostedChatTopic[] = projects.map((project) => ({
    room_id: ROOM_ID,
    topic_id: topicIdForProject(project),
    title: project.label,
    description: project.path,
    last_message_preview: project.previewSessions[0]?.preview ?? "",
    unread_count: 0,
    message_count: project.sessionCount,
    created_seq: 0,
    updated_seq: 0,
    archived: false,
    active_chat_id: null,
    chats: [
      ...project.previewSessions.map(summaryFromSession),
      ...[...chats.values()]
        .filter((entry) => entry.topicId === topicIdForProject(project))
        .map((entry) => entry.summary),
    ],
  }));
  const recentsChats = [
    ...sessions.filter((session) => !scoped.has(session.id)).map(summaryFromSession),
    ...[...chats.values()]
      .filter((entry) => entry.topicId === RECENTS_TOPIC_ID)
      .map((entry) => entry.summary),
  ];
  topics.push({
    room_id: ROOM_ID,
    topic_id: RECENTS_TOPIC_ID,
    title: "Recents",
    description: "Sessions no project claims",
    last_message_preview: recentsChats[0]?.last_message_preview ?? "",
    unread_count: 0,
    message_count: recentsChats.length,
    created_seq: 0,
    updated_seq: 0,
    archived: false,
    active_chat_id: null,
    chats: recentsChats,
  });
  return topics;
}

function gatewayMessage(
  role: string,
  text: string,
  chatId: string,
  historical: boolean,
  status: "running" | "complete" = "complete"
): HostedChatMessage {
  const mine = role === "user";
  const seconds = Math.floor(Date.now() / 1000);
  return {
    room_id: ROOM_ID,
    seq: 0,
    message_id: `${chatId}:${role}:${seconds}:${Math.random().toString(36).slice(2, 8)}`,
    conversation_id: null,
    chat_id: chatId,
    sender_account_id: mine ? MY_ACCOUNT : AGENT_ACCOUNT,
    sender_device_id: mine ? "web-gateway" : "gateway",
    sender_display_name: mine ? "You" : AGENT_NAME,
    sender_npub: null,
    text,
    display_content: text,
    rich_text_json: undefined,
    metadata_json: undefined,
    reply_to_message_id: null,
    is_mine: mine,
    outbound_delivery: mine ? { local_send: "Sent", server_delivery: "Delivered" } : null,
    media: [],
    kind: "message",
    status,
    final_delivery: status === "complete" || historical,
    edit_of_message_id: null,
    timestamp_unix_seconds: seconds,
    display_timestamp: new Date(seconds * 1000).toLocaleTimeString([], {
      hour: "numeric",
      minute: "2-digit",
    }),
    ...(historical ? {} : {}),
  };
}
