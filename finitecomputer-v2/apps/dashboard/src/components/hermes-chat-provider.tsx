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
// Gated mode (password-protected gateway): login once, then mint a
// single-use 30s ws ticket per connection (hermes: POST /auth/password-login
// then POST /api/auth/ws-ticket). Cross-origin cookies need a same-origin
// path — use the dev rewrite (see next.config.ts) and point
// GATEWAY_WS_URL at it.
const GATEWAY_USERNAME = process.env.NEXT_PUBLIC_HERMES_GATEWAY_USERNAME || "";
const GATEWAY_PASSWORD = process.env.NEXT_PUBLIC_HERMES_GATEWAY_PASSWORD || "";

function gatewayHttpBase() {
  return GATEWAY_WS_URL.replace(/^ws:/u, "http:").replace(/^wss:/u, "https:").replace(/\/api\/ws\/?$/u, "");
}

async function mintWsTarget(): Promise<string> {
  if (!GATEWAY_USERNAME) {
    return GATEWAY_TOKEN
      ? `${GATEWAY_WS_URL}?token=${encodeURIComponent(GATEWAY_TOKEN)}`
      : GATEWAY_WS_URL;
  }
  const base = gatewayHttpBase();
  const providers = await fetch(`${base}/api/auth/providers`, {
    credentials: "include",
  }).then(
    (response) =>
      response.json() as Promise<
        { providers?: { id?: string; name?: string; supports_password?: boolean }[] } | null
      >
  );
  const passwordProvider = providers?.providers?.find(
    (provider) => provider.supports_password
  );
  if (!passwordProvider) throw new Error("gateway has no password auth provider");
  const login = await fetch(`${base}/auth/password-login`, {
    method: "POST",
    credentials: "include",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({
      provider: passwordProvider.id ?? passwordProvider.name ?? "",
      username: GATEWAY_USERNAME,
      password: GATEWAY_PASSWORD,
      next: "",
    }),
  });
  if (!login.ok) throw new Error(`gateway login failed (${login.status})`);
  const ticket = await fetch(`${base}/api/auth/ws-ticket`, {
    method: "POST",
    credentials: "include",
  }).then((response) => response.json() as Promise<{ ticket?: string } | null>);
  if (!ticket?.ticket) throw new Error("gateway returned no ws ticket");
  return `${GATEWAY_WS_URL}?ticket=${encodeURIComponent(ticket.ticket)}`;
}

const ROOM_ID = "hermes";
// hermes' own sidebar model: projects.tree claims sessions (scoped_session_ids)
// and everything left belongs in a flat Recents bucket. Rendered even when
// empty so New chat always has a home; new chats sent from Recents carry no
// cwd and hermes categorizes them on the first prompt.
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

// hermes history rows are richer than user/assistant prose: tool calls carry
// name/context, assistant rows carry their reasoning.
type GatewayMessage = {
  role: string;
  text?: string;
  name?: string;
  context?: string;
  reasoning?: string;
};

type Streaming = { turnKey: number; reasoning: string; answer: string };

/** A chat the UI knows about: a persisted gateway session or a local draft. */
type ChatEntry = {
  summary: HostedChatSummary;
  /** Short gateway handle for RPCs; drafts keep their session.create id. */
  handleId: string;
  /** The stored id session.create reports for drafts; null once persisted. */
  storedId: string | null;
  /** Project id whose cwd spawned it (drives StartTopicChatIntent). */
  topicId: string;
  /** Present while a turn is streaming onto this chat. */
  streaming: Streaming | null;
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
    topicId: RECENTS_TOPIC_ID,
    chatId: null,
  });
  const turnKeyRef = useRef(0);
  const socketRef = useRef<WebSocket | null>(null);
  const nextIdRef = useRef(1);
  const pendingRef = useRef(
    new Map<number, { resolve: (value: unknown) => void; reject: (reason: Error) => void; timer: ReturnType<typeof setTimeout> }>()
  );
  const callRef = useRef<GatewayCall | null>(null);
  const selectionSequenceRef = useRef(0);
  const listSequenceRef = useRef(0);

  const stateRef = useRef<HostedChatState | null>(null);

  // Dispatch callers get the freshest published snapshot.
  const currentState = useCallback(
    (): HostedChatState =>
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
      },
    []
  );

  const publish = useCallback(() => {
    const topics = topicsFrom(
      projectsRef.current,
      sessionsRef.current,
      scopedRef.current,
      chatsRef.current
    );
    const selected = selectedRef.current;
    // The chat's CURRENT home topic — hermes can move a chat between topics
    // (draft in Recents → project) and messages must follow the chat.
    const selectedTopicId = (selected.chatId ? chatsRef.current.get(selected.chatId)?.topicId : null)
      ?? selected.topicId;
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
      selected_topic_id: selectedTopicId,
      selected_chat_id: selected.chatId,
      active_profile_id: null,
      status: "ok",
      toast: null,
      messages: selected.chatId
        ? (transcriptRef.current.get(selected.chatId) ?? []).map((message) =>
            message.conversation_id === selectedTopicId
              ? message
              : { ...message, conversation_id: selectedTopicId }
          )
        : [],
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
      typing_members: Boolean(selected.chatId && chatsRef.current.get(selected.chatId)?.streaming)
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


  /**
   * Live turn rows: the reasoning stream rides a kind:"tool" message so the
   * shared transcript groups it into the collapsed ToolRollup ("Working ·
   * N steps", expandable, auto-open while running) and the reply is the
   * only prose bubble — the two can never bleed into each other. Stable
   * per-turn ids let React reconcile updates in place.
   */
  const upsertStreamingMessages = useCallback((chatId: string, topicId: string, streaming: Streaming) => {
    const messages = transcriptRef.current.get(chatId) ?? [];
    const upsert = (message: HostedChatMessage) => {
      const index = messages.findIndex(
        (candidate) => candidate.message_id === message.message_id
      );
      if (index >= 0) messages.splice(index, 1, message);
      else messages.push(message);
    };
    if (streaming.reasoning) {
      upsert({
        ...gatewayMessage("assistant", streaming.reasoning, chatId, topicId, false, "running"),
        message_id: `gateway:think:${streaming.turnKey}`,
        kind: "tool",
        display_content: streaming.reasoning,
      });
    }
    if (streaming.answer) {
      upsert({
        ...gatewayMessage("assistant", streaming.answer, chatId, topicId, false, "running"),
        message_id: `gateway:reply:${streaming.turnKey}`,
      });
    }
    transcriptRef.current.set(chatId, messages);
  }, []);

  // Reopening and reconnecting bind a fresh transport to durable history.
  const hydrateTranscript = useCallback(async (chatId: string, topicId: string) => {
    const call = callRef.current;
    if (!call) return false;
    try {
      const resumed = (await call("session.resume", { session_id: chatId })) as {
        session_id: string;
        messages?: GatewayMessage[];
      } | null;
      if (call !== callRef.current) return false;
      if (!resumed?.session_id) throw new Error("Gateway returned no session handle");
      transcriptRef.current.set(chatId, historyMessageRows(resumed.messages ?? [], chatId, topicId));
      const entry = chatsRef.current.get(chatId);
      if (entry) entry.handleId = resumed.session_id;
      publish();
    } catch {
      setTransportError("Could not reload this conversation. Retry before sending.");
      return false;
    }
    return true;
  }, [publish]);

  const refreshLists = useCallback(async () => {
    const call = callRef.current;
    if (!call) return;
    const sequence = ++listSequenceRef.current;
    const [treeResult, listResult] = await Promise.all([
      call("projects.tree", { preview_limit: 50 }),
      call("session.list", { limit: 100 }),
    ]);
    if (sequence !== listSequenceRef.current || call !== callRef.current) return;
    const tree = treeResult as { projects?: GatewayProject[]; scoped_session_ids?: string[] } | null;
    const list = listResult as { sessions?: GatewaySession[] } | null;
    projectsRef.current = (tree?.projects ?? []).filter((project) => project.sessionCount > 0);
    scopedRef.current = new Set(tree?.scoped_session_ids ?? []);
    const sessions = list?.sessions ?? [];
    sessionsRef.current = sessions;
    // Adopt persisted sessions, each under the project that claims it.
    for (const session of sessions) {
      const existing = chatsRef.current.get(session.id);
      chatsRef.current.set(session.id, {
        summary: summaryFromSession(session),
        handleId: existing?.handleId ?? "",
        storedId: null,
        topicId: topicOfSession(session, projectsRef.current),
        streaming: existing?.streaming ?? null,
      });
    }
    // Reconcile drafts: the moment a draft's stored id materializes as a
    // real session, replace the local row with hermes' row (title, project)
    // and carry selection, handle, transcript, and stream state across.
    for (const [chatId, entry] of [...chatsRef.current]) {
      if (!chatId.startsWith("draft:") || !entry.storedId) continue;
      const real = sessions.find((session) => session.id === entry.storedId);
      if (!real) continue;
      chatsRef.current.delete(chatId);
      const transcript = transcriptRef.current.get(chatId);
      if (transcript) {
        transcriptRef.current.set(real.id, transcript.map((message) => ({ ...message, chat_id: real.id })));
        transcriptRef.current.delete(chatId);
      }
      const topicId = topicOfSession(real, projectsRef.current);
      chatsRef.current.set(real.id, {
        summary: summaryFromSession(real),
        handleId: entry.handleId,
        storedId: null,
        topicId,
        streaming: entry.streaming,
      });
      if (selectedRef.current.chatId === chatId) {
        selectedRef.current = { topicId, chatId: real.id };
      }
      // Keep the in-flight tail intact; reopening reconciles with history.
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
      if (type === "session.reclaimed") {
        // hermes reaped an orphaned draft; drop our local row for it.
        const stored = String(event.payload?.stored_session_id ?? "");
        if (!stored) return;
        for (const [chatId, entry] of [...chatsRef.current]) {
          if (entry.storedId === stored) {
            chatsRef.current.delete(chatId);
            transcriptRef.current.delete(chatId);
            if (selectedRef.current.chatId === chatId) {
              selectedRef.current = { topicId: null, chatId: null };
            }
          }
        }
        publish();
        return;
      }
      const chat = [...chatsRef.current.values()].find(
        (candidate) => event.session_id && candidate.handleId === event.session_id
      );
      if (!chat) return;
      const chatId = chat.summary.chat_id;
      if (type === "message.start") {
        chat.streaming = { turnKey: ++turnKeyRef.current, reasoning: "", answer: "" };
        publish();
      } else if (type === "reasoning.delta" || type === "message.delta") {
        if (chat.streaming) {
          if (type === "reasoning.delta") {
            chat.streaming.reasoning += String(event.payload?.text ?? "");
          } else {
            chat.streaming.answer += String(event.payload?.text ?? "");
          }
          upsertStreamingMessages(chatId, chat.topicId, chat.streaming);
          publish();
        }
      } else if (type === "message.complete") {
        const text = String(event.payload?.text ?? "");
        chat.streaming = null;
        const messages = transcriptRef.current.get(chatId) ?? [];
        // The reply row is replaced by the final text; every live rollup row
        // (the thinking trace) settles to complete.
        const replyIndex = messages.findIndex(
          (message) => message.status === "running" && message.kind !== "tool"
        );
        const finalMessage = gatewayMessage("assistant", text, chatId, chat.topicId, false);
        if (replyIndex >= 0) messages.splice(replyIndex, 1, finalMessage);
        else if (text) messages.push(finalMessage);
        for (const message of messages) {
          if (message.status === "running") {
            message.status = "complete";
            message.final_delivery = true;
          }
        }
        transcriptRef.current.set(chatId, messages);
        publish();
      }
    },
    [publish, refreshLists, upsertStreamingMessages]
  );

  useEffect(() => {
    let disposed = false;
    const pendingRequests = pendingRef.current;
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
        const timer = setTimeout(() => {
          pendingRequests.delete(id);
          reject(new Error("Gateway request timed out. Check the conversation before retrying."));
        }, 30_000);
        pendingRequests.set(id, { resolve, reject, timer });
        try {
          socket.send(JSON.stringify({ jsonrpc: "2.0", id, method, params }));
        } catch (error) {
          clearTimeout(timer);
          pendingRequests.delete(id);
          reject(error);
        }
      });
    callRef.current = call;

    const connect = () => {
      if (disposed) return;
      void mintWsTarget()
        .then((target) => {
          if (disposed) return;
          openSocket(target);
        })
        .catch(() => {
          if (disposed) return;
          setStreamConnected(false);
          setTransportError(`gateway unreachable at ${GATEWAY_WS_URL}`);
          retryTimer = setTimeout(connect, retryMs);
          retryMs = Math.min(retryMs * 2, 10_000);
        });
    };

    const openSocket = (target: string) => {
      if (disposed) return;
      const socket = new WebSocket(target);
      socketRef.current = socket;

      socket.addEventListener("open", () => {
        if (disposed || socketRef.current !== socket) return;
        retryMs = 500;
        setStreamConnected(true);
        setTransportError(null);
        void refreshLists().then(async () => {
          const selected = selectedRef.current;
          if (selected.chatId && !selected.chatId.startsWith("draft:")) {
            await hydrateTranscript(selected.chatId, selected.topicId ?? RECENTS_TOPIC_ID);
          }
        }).catch(() => setTransportError("Could not load gateway conversations. Retry load to reconnect."));
      });

      socket.addEventListener("message", (event: MessageEvent) => {
        if (disposed || socketRef.current !== socket) return;
        let message: GatewayInbound;
        try {
          message = JSON.parse(String(event.data));
        } catch {
          return;
        }
        if (!message || typeof message !== "object") return;
        if (message.method === "event" && message.params?.type) {
          handleEvent(message.params);
          return;
        }
        if (message.id == null) return;
        const pending = pendingRequests.get(message.id);
        if (!pending) return;
        clearTimeout(pending.timer);
        pendingRequests.delete(message.id);
        if (message.error) {
          pending.reject(
            new Error(`${message.error.message ?? "gateway error"} (${message.error.code ?? "?"})`)
          );
        } else {
          pending.resolve(message.result);
        }
      });

      const reconnect = () => {
        if (disposed || socketRef.current !== socket) return;
        socketRef.current = null;
        ++listSequenceRef.current;
        for (const [chatId, entry] of chatsRef.current) {
          entry.handleId = "";
          entry.streaming = null;
          if (chatId.startsWith("draft:") && !entry.storedId) {
            chatsRef.current.delete(chatId);
          }
        }
        for (const pending of pendingRequests.values()) {
          clearTimeout(pending.timer);
          pending.reject(new Error("Gateway connection lost. Check the conversation before retrying."));
        }
        pendingRequests.clear();
        setStreamConnected(false);
        setTransportError(`gateway unreachable at ${GATEWAY_WS_URL}`);
        retryTimer = setTimeout(connect, retryMs);
        retryMs = Math.min(retryMs * 2, 10_000);
      };
      socket.addEventListener("close", reconnect);
      // Browsers emit close after error; scheduling from both duplicates sockets.
      socket.addEventListener("error", () => socket.close());
    };

    connect();
    return () => {
      disposed = true;
      if (retryTimer) clearTimeout(retryTimer);
      const socket = socketRef.current;
      socketRef.current = null;
      callRef.current = null;
      for (const pending of pendingRequests.values()) {
        clearTimeout(pending.timer);
        pending.reject(new Error("Gateway connection closed"));
      }
      pendingRequests.clear();
      socket?.close();
    };
  }, [handleEvent, hydrateTranscript, refreshLists]);

  const createDraft = useCallback(async (topic_id: string) => {
    const call = callRef.current;
    if (!call) throw new Error("Gateway is not connected");
    const project = projectsRef.current.find(
      (candidate) => topicIdForProject(candidate) === topic_id
    );
    // source "desktop" is the client class we actually are: a remote UI
    // with no launch folder of its own, exactly like the desktop app.
    // hermes stamps no workspace on unpicked desktop-class creates, so
    // they join the Home (no-project) bucket instead of inheriting the
    // gateway's launch directory. A cwd is passed only when the user
    // picked a real project topic.
    const created = (await call("session.create", {
      source: "desktop",
      cols: 100,
      ...(project?.path ? { cwd: project.path } : {}),
    })) as { session_id: string; stored_session_id?: string } | null;
    if (!created?.session_id) throw new Error("Gateway returned no session handle");
    const draftId = `draft:${created.session_id}`;
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
      storedId: created?.stored_session_id ?? null,
      topicId: topic_id,
      streaming: null,
    });
    transcriptRef.current.set(draftId, []);
    selectedRef.current = { topicId: topic_id, chatId: draftId };
    return draftId;
  }, []);

  const dispatch = useCallback(
    async (action: HostedChatAction): Promise<HostedChatState> => {
      const call = callRef.current;
      if (!call) throw new Error("hermes gateway is not connected");

      if ("OpenChat" in action) {
        const { chat_id } = action.OpenChat;
        const sequence = ++selectionSequenceRef.current;
        selectedRef.current = {
          topicId: action.OpenChat.topic_id,
          chatId: chat_id,
        };
        let entry = chatsRef.current.get(chat_id);
        if (!entry) {
          const preview = projectsRef.current.flatMap((project) => project.previewSessions).find((session) => session.id === chat_id);
          if (!preview) throw new Error("Conversation is unavailable. Reload the list.");
          entry = { summary: summaryFromSession(preview), handleId: "", storedId: null, topicId: action.OpenChat.topic_id, streaming: null };
          chatsRef.current.set(chat_id, entry);
        }
        if (entry) entry.topicId = action.OpenChat.topic_id;
        if (entry && !chat_id.startsWith("draft:")) {
          const resumed = (await call("session.resume", { session_id: chat_id })) as {
            session_id: string;
            messages?: GatewayMessage[];
          } | null;
          if (sequence !== selectionSequenceRef.current) return currentState();
          if (!resumed?.session_id) throw new Error("Gateway returned no session handle");
          entry.handleId = resumed.session_id;
          transcriptRef.current.set(
            chat_id,
            historyMessageRows(resumed?.messages ?? [], chat_id, entry.topicId)
          );
        }
        publish();
        return currentState();
      }

      if ("OpenTopic" in action) {
        ++selectionSequenceRef.current;
        selectedRef.current = { topicId: action.OpenTopic.topic_id, chatId: null };
        publish();
        return currentState();
      }

      if ("StartTopicChatIntent" in action) {
        await createDraft(action.StartTopicChatIntent.topic_id);
        publish();
        return currentState();
      }

      if ("CreateTopic" in action) {
        await call("projects.create", { name: action.CreateTopic.title });
        await refreshLists();
        return currentState();
      }

      // The real composer sends SendChatMessage when a chat is selected,
      // SendTopicMessage when only a topic is, and SendMessage for bare
      // rooms — all three land on prompt.submit here.
      if ("SendChatMessage" in action || "SendTopicMessage" in action || "SendMessage" in action) {
        const chatScoped = "SendChatMessage" in action ? action.SendChatMessage : null;
        const text =
          chatScoped?.text
          ?? ("SendTopicMessage" in action
            ? action.SendTopicMessage.text
            : "SendMessage" in action
              ? action.SendMessage.text
              : "");
        const explicitChatId = chatScoped?.chat_id ?? null;
        let chatId = explicitChatId ?? selectedRef.current.chatId;
        const topicId = chatScoped?.topic_id
          ?? ("SendTopicMessage" in action ? action.SendTopicMessage.topic_id : selectedRef.current.topicId)
          ?? RECENTS_TOPIC_ID;
        if (explicitChatId && !chatsRef.current.has(explicitChatId)) {
          throw new Error("Conversation is unavailable. Reopen it before sending.");
        }
        if (!chatId) chatId = await createDraft(topicId);
        const entry = chatsRef.current.get(chatId);
        if (!entry) throw new Error("Conversation is unavailable. Reopen it before sending.");
        if (!entry.handleId) {
          if (chatId.startsWith("draft:")) {
            throw new Error("This unsent draft lost its connection. Start a new chat to send it.");
          }
          const resumed = await call("session.resume", { session_id: chatId }) as { session_id?: string };
          if (!resumed?.session_id) throw new Error("Gateway returned no session handle");
          entry.handleId = resumed.session_id;
        }
        const handle = entry.handleId;
        const messages = transcriptRef.current.get(chatId) ?? [];
        const optimistic = gatewayMessage("user", text, chatId, entry.topicId, false);
        optimistic.outbound_delivery = { local_send: "Sending", server_delivery: "Undelivered" };
        messages.push(optimistic);
        transcriptRef.current.set(chatId, messages);
        // Publish the optimistic user message BEFORE awaiting the turn so a
        // slow gateway can never freeze the composer.
        publish();
        try {
          await call("prompt.submit", { session_id: handle, text });
          for (const rows of transcriptRef.current.values()) {
            const delivered = rows.find((row) => row.message_id === optimistic.message_id);
            if (delivered) delivered.outbound_delivery = { local_send: "Sent", server_delivery: "Delivered" };
          }
        } catch (error) {
          // The composer retains the text. Never leave a refused send marked delivered.
          for (const rows of transcriptRef.current.values()) {
            const index = rows.findIndex((row) => row.message_id === optimistic.message_id);
            if (index >= 0) rows.splice(index, 1);
          }
          publish();
          throw error;
        }
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

      // Read receipts and outgoing typing have no gateway equivalent.
      // Do not publish for these effects or they would dispatch in a loop.
      // Other unsupported mutations must fail visibly.
      if (!("MarkRoomRead" in action) && !("SetTyping" in action)) {
        throw new Error("This action is not supported by gateway chat yet.");
      }
      return currentState();
    },
    [createDraft, currentState, publish, refreshLists]
  );

  const load = useCallback(async (): Promise<HostedChatRetryAttempt> => {
    try {
      await refreshLists();
      const selected = selectedRef.current;
      if (selected.chatId && !selected.chatId.startsWith("draft:")) {
        const loaded = await hydrateTranscript(selected.chatId, selected.topicId ?? RECENTS_TOPIC_ID);
        if (!loaded) return "stop";
      }
      setTransportError(null);
      return "succeeded";
    } catch {
      setTransportError(`gateway unreachable at ${GATEWAY_WS_URL}`);
      return "stop";
    }
  }, [hydrateTranscript, refreshLists]);

  const claimOwner = useCallback(
    async (): Promise<HostedChatRetryAttempt> => "succeeded",
    []
  );
  const recoverBinding = useCallback(
    async (): Promise<HostedChatRetryAttempt> => "succeeded",
    []
  );
  const dispatchQuiet = useCallback(async (action: HostedChatAction) => {
    try {
      return await dispatch(action);
    } catch {
      return null;
    }
  }, [dispatch]);
  const refreshPendingChat = useCallback(async (target: PendingChatRefreshTarget) => {
    if (chatsRef.current.get(target.chat_id)?.streaming) return false;
    return await hydrateTranscript(target.chat_id, target.topic_id);
  }, [hydrateTranscript]);
  const uploadAttachments = useCallback(async (): Promise<HostedChatState> => {
    throw new Error("attachments are not wired to the gateway yet");
  }, []);

  const value = useMemo<HostedChatContextValue>(
    () => ({
      apiBase: "hermes-gateway",
      canSendToTopic: true,
      supportsAttachments: false,
      supportsChatArchive: false,
      state,
      transportError,
      claimError: null,
      streamConnected,
      ownerClaimed: true,
      bindingRecoveryRequired: false,
      selectionPending: false,
      load,
      claimOwner,
      recoverBinding,
      dispatch,
      dispatchQuiet,
      refreshPendingChat,
      uploadAttachments,
      attachmentUrl: () => "#",
    }),
    [claimOwner, dispatch, dispatchQuiet, load, recoverBinding, refreshPendingChat, state, streamConnected, transportError, uploadAttachments]
  );


  return <HostedChatContext.Provider value={value}>{children}</HostedChatContext.Provider>;
}

// chatsRef entries and hermes' previewSessions describe the same sessions;
// local entries win (they carry live handles) and hermes' rows fill the rest.
function uniqueChats(...sources: HostedChatSummary[][]): HostedChatSummary[] {
  const byId = new Map<string, HostedChatSummary>();
  for (const source of sources) {
    for (const summary of source) {
      if (!byId.has(summary.chat_id)) byId.set(summary.chat_id, summary);
    }
  }
  return [...byId.values()];
}

/**
 * Map hermes history rows onto transcript rows: user/assistant prose stay
 * prose; tool calls and assistant reasoning ride kind:"tool" rows so the
 * shared transcript collapses them into the ToolRollup; rows with nothing
 * to show (empty markers) are dropped instead of becoming blank bubbles.
 */
function historyMessageRows(
  rows: GatewayMessage[],
  chatId: string,
  topicId: string
): HostedChatMessage[] {
  const mapped: HostedChatMessage[] = [];
  let step = 0;
  for (const row of rows) {
    if (row.role === "user" && row.text) {
      mapped.push(gatewayMessage("user", row.text, chatId, topicId, true));
    } else if (row.role === "assistant") {
      if (row.reasoning) {
        step += 1;
        mapped.push({
          ...gatewayMessage("assistant", row.reasoning, chatId, topicId, true),
          message_id: `${chatId}:think:h${step}`,
          kind: "tool",
          display_content: row.reasoning,
        });
      }
      if (row.text) {
        mapped.push(gatewayMessage("assistant", row.text, chatId, topicId, true));
      }
    } else if (row.role === "tool") {
      const label = [row.name, row.context].filter(Boolean).join(": ");
      const content = row.text || label;
      if (!content) continue;
      step += 1;
      mapped.push({
        ...gatewayMessage("assistant", content, chatId, topicId, true),
        message_id: `${chatId}:tool:h${step}`,
        kind: "tool",
        display_content: content,
      });
    } else if (row.text) {
      mapped.push(gatewayMessage(row.role, row.text, chatId, topicId, true));
    }
  }
  return mapped;
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

function topicOfSession(session: GatewaySession, projects: GatewayProject[]): string {
  const project = projects.find((candidate) =>
    candidate.previewSessions.some((candidateSession) => candidateSession.id === session.id)
  );
  return project ? topicIdForProject(project) : RECENTS_TOPIC_ID;
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
    chats: uniqueChats(
      [...chats.values()]
        .filter((entry) => entry.topicId === topicIdForProject(project))
        .map((entry) => entry.summary),
      project.previewSessions.map(summaryFromSession)
    ),
  }));
  const recentsChats = uniqueChats(
    [...chats.values()]
      .filter((entry) => entry.topicId === RECENTS_TOPIC_ID)
      .map((entry) => entry.summary),
    sessions.filter((session) => !scoped.has(session.id)).map(summaryFromSession)
  );
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
  topicId: string,
  historical: boolean,
  status: "running" | "complete" = "complete"
): HostedChatMessage {
  const mine = role === "user";
  const seconds = Math.floor(Date.now() / 1000);
  return {
    room_id: ROOM_ID,
    seq: 0,
    message_id: `${chatId}:${role}:${seconds}:${Math.random().toString(36).slice(2, 8)}`,
    // The real transcript renders only messages whose conversation_id
    // matches the selected topic's id.
    conversation_id: topicId,
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
