"use client";

/**
 * Admin preview: the existing chat UI over the agent's native Hermes server.
 *
 * Supplies the same context as the Finite Chat hosted-device provider so
 * AgentSidebar and HostedWebChat render unchanged. History comes from
 * Hermes's own session store over REST (archived sessions included, paged);
 * live turns use the native `/api/ws` JSON-RPC gateway with a Core-issued
 * grant and a single-use ticket per connection. No credential is cached
 * outside one connect or read operation.
 */

import type { ReactNode } from "react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { HostedChatContext, type HostedChatContextValue } from "@/components/hosted-chat-provider";
import {
  HERMES_AGENT_ACCOUNT,
  HERMES_ROOM_ID,
  HERMES_VIEWER_ACCOUNT,
  hermesMessage,
  hermesSessionReadOnly,
  hermesToolRow,
  hermesTopicGroups,
  hermesTopicIdForSession,
  hermesTranscript,
  parseHermesMessages,
  parseHermesSessionPage,
  type HermesSessionRow,
} from "@/lib/hermes-chat-sessions";
import { mintHostedHermesGatewaySocket, readHostedHermesJson } from "@/lib/hosted-hermes-status";
import type { HostedChatRetryAttempt } from "@/lib/hosted-web-chat-retry";
import type { PendingChatRefreshTarget } from "@/lib/hosted-web-chat-refresh";
import { HOME_TOPIC_ID } from "@/lib/hosted-web-chat-topics";
import type {
  HostedChatAction,
  HostedChatMessage,
  HostedChatState,
  HostedChatSummary,
} from "@/lib/hosted-web-device";

const SESSION_PAGE_SIZE = 100;
const MAX_SESSION_PAGES = 20;
const TRANSCRIPT_PAGE_SIZE = 200;
const TRANSCRIPT_MAX_BYTES = 8 * 1024 * 1024;
const RPC_TIMEOUT_MS = 30_000;
const READ_ONLY_REASON =
  "This chat came in through another app. It is read-only here; start a new chat to talk on the web.";

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

type Streaming = { turnKey: number; reasoning: string; answer: string };

/** A web chat Hermes has not listed yet (no first prompt). */
type Draft = { summary: HostedChatSummary; handleId: string; storedId: string | null };

type GatewayCall = (method: string, params?: Record<string, unknown>) => Promise<unknown>;

export function HermesChatProvider({
  children,
  runtimeId,
  agentName,
}: {
  children: ReactNode;
  runtimeId: string;
  agentName: string;
}) {
  const [state, setState] = useState<HostedChatState | null>(null);
  const [transportError, setTransportError] = useState<string | null>(null);
  const [streamConnected, setStreamConnected] = useState(false);

  const revRef = useRef(1);
  const sessionsRef = useRef<HermesSessionRow[]>([]);
  const draftsRef = useRef(new Map<string, Draft>());
  const handlesRef = useRef(new Map<string, string>()); // stored session id → live gateway handle
  const streamingRef = useRef(new Map<string, Streaming>()); // chat id → live turn
  const transcriptRef = useRef(new Map<string, HostedChatMessage[]>());
  const selectedRef = useRef<{ topicId: string | null; chatId: string | null }>({
    topicId: HOME_TOPIC_ID,
    chatId: null,
  });
  const turnKeyRef = useRef(0);
  const socketRef = useRef<WebSocket | null>(null);
  const callRef = useRef<GatewayCall | null>(null);
  const listSequenceRef = useRef(0);
  const selectionSequenceRef = useRef(0);
  const stateRef = useRef<HostedChatState | null>(null);
  const lifetimeRef = useRef(new AbortController());

  const currentState = useCallback((): HostedChatState => stateRef.current ?? emptyState(), []);

  const sessionFor = useCallback(
    (chatId: string | null) => (chatId ? sessionsRef.current.find((session) => session.id === chatId) ?? null : null),
    []
  );

  const topicOf = useCallback((chatId: string) => {
    if (draftsRef.current.has(chatId)) return HOME_TOPIC_ID;
    const session = sessionFor(chatId);
    return session ? hermesTopicIdForSession(session) : HOME_TOPIC_ID;
  }, [sessionFor]);

  const publish = useCallback(() => {
    const groups = hermesTopicGroups(
      sessionsRef.current,
      [...draftsRef.current.values()].map((draft) => draft.summary)
    );
    const selected = selectedRef.current;
    const selectedTopicId = selected.chatId ? topicOf(selected.chatId) : selected.topicId;
    const streaming = selected.chatId ? streamingRef.current.get(selected.chatId) : undefined;
    const next: HostedChatState = {
      ...emptyState(),
      rev: revRef.current++,
      rooms: [{
        room_id: HERMES_ROOM_ID,
        display_name: agentName,
        state: "Connected",
        status: "ok",
        user_status_text: "",
        last_message_preview: sessionsRef.current[0]?.preview ?? "",
        unread_count: 0,
        can_load_older: false,
        is_agent_chat: true,
      }],
      topics: groups.map((group) => group.topic),
      selected_room_id: HERMES_ROOM_ID,
      selected_topic_id: selectedTopicId,
      selected_chat_id: selected.chatId,
      messages: selected.chatId ? transcriptRef.current.get(selected.chatId) ?? [] : [],
      profiles: [{
        account_id: HERMES_AGENT_ACCOUNT,
        npub: "",
        display_name: agentName,
        about: null,
        picture: null,
        stale: false,
        is_agent: true,
      }],
      typing_members: streaming
        ? [{
            room_id: HERMES_ROOM_ID,
            topic_id: selectedTopicId,
            chat_id: selected.chatId,
            account_id: HERMES_AGENT_ACCOUNT,
            device_id: "hermes",
            display_name: agentName,
            activity_kind: "thinking",
          }]
        : [],
    };
    stateRef.current = next;
    setState(next);
  }, [agentName, topicOf]);

  const refreshSessions = useCallback(async () => {
    const sequence = ++listSequenceRef.current;
    const signal = lifetimeRef.current.signal;
    const sessions: HermesSessionRow[] = [];
    for (let page = 0; page < MAX_SESSION_PAGES; page++) {
      const result = parseHermesSessionPage(await readHostedHermesJson(runtimeId, "api/sessions", signal, {
        query: {
          archived: "include",
          order: "recent",
          limit: SESSION_PAGE_SIZE,
          offset: page * SESSION_PAGE_SIZE,
        },
        maxBytes: 4 * 1024 * 1024,
      }));
      sessions.push(...result.sessions);
      if (result.sessions.length < SESSION_PAGE_SIZE || sessions.length >= result.total) break;
    }
    if (sequence !== listSequenceRef.current) return;
    sessionsRef.current = sessions;
    // A draft becomes a listed session on its first prompt: carry selection,
    // handle, transcript and stream state onto the stored id.
    for (const [draftId, draft] of [...draftsRef.current]) {
      if (!draft.storedId || !sessions.some((session) => session.id === draft.storedId)) continue;
      draftsRef.current.delete(draftId);
      const storedId = draft.storedId;
      handlesRef.current.set(storedId, draft.handleId);
      const transcript = transcriptRef.current.get(draftId);
      if (transcript) {
        transcriptRef.current.set(storedId, transcript.map((message) => ({ ...message, chat_id: storedId })));
        transcriptRef.current.delete(draftId);
      }
      const streaming = streamingRef.current.get(draftId);
      if (streaming) {
        streamingRef.current.set(storedId, streaming);
        streamingRef.current.delete(draftId);
      }
      if (selectedRef.current.chatId === draftId) {
        selectedRef.current = { topicId: HOME_TOPIC_ID, chatId: storedId };
      }
    }
    publish();
  }, [publish, runtimeId]);

  const loadTranscript = useCallback(async (chatId: string) => {
    const result = parseHermesMessages(await readHostedHermesJson(
      runtimeId,
      `api/sessions/${chatId}/messages`,
      lifetimeRef.current.signal,
      // Compaction retires rows from the model's context, not from history.
      { query: { order: "latest", limit: TRANSCRIPT_PAGE_SIZE, include_compacted: true }, maxBytes: TRANSCRIPT_MAX_BYTES }
    ));
    if (streamingRef.current.has(chatId)) return;
    transcriptRef.current.set(chatId, hermesTranscript(result, chatId, topicOf(chatId), agentName));
  }, [agentName, runtimeId, topicOf]);

  const upsertStreaming = useCallback((chatId: string, streaming: Streaming) => {
    const messages = transcriptRef.current.get(chatId) ?? [];
    const topicId = topicOf(chatId);
    const upsert = (message: HostedChatMessage) => {
      const index = messages.findIndex((candidate) => candidate.message_id === message.message_id);
      if (index >= 0) messages.splice(index, 1, message);
      else messages.push(message);
    };
    if (streaming.reasoning) {
      upsert(hermesToolRow(streaming.reasoning, `live:think:${streaming.turnKey}`, chatId, topicId, agentName, "running"));
    }
    if (streaming.answer) {
      upsert(hermesMessage({
        role: "assistant",
        text: streaming.answer,
        chatId,
        topicId,
        id: `live:reply:${streaming.turnKey}`,
        at: Date.now() / 1000,
        agentName,
        status: "running",
      }));
    }
    transcriptRef.current.set(chatId, messages);
  }, [agentName, topicOf]);

  const chatForHandle = useCallback((handle: string | undefined) => {
    if (!handle) return null;
    for (const [draftId, draft] of draftsRef.current) {
      if (draft.handleId === handle) return draftId;
    }
    for (const [storedId, candidate] of handlesRef.current) {
      if (candidate === handle) return storedId;
    }
    return null;
  }, []);

  const handleEvent = useCallback((event: GatewayEvent) => {
    const type = event.type ?? "";
    if (type === "sessions.changed") {
      void refreshSessions().catch(() => undefined);
      return;
    }
    if (type === "session.reclaimed") {
      const stored = String(event.payload?.stored_session_id ?? "");
      for (const [draftId, draft] of [...draftsRef.current]) {
        if (draft.storedId !== stored) continue;
        draftsRef.current.delete(draftId);
        transcriptRef.current.delete(draftId);
        if (selectedRef.current.chatId === draftId) {
          selectedRef.current = { topicId: HOME_TOPIC_ID, chatId: null };
        }
      }
      publish();
      return;
    }
    const chatId = chatForHandle(event.session_id);
    if (!chatId) return;
    if (type === "message.start") {
      streamingRef.current.set(chatId, { turnKey: ++turnKeyRef.current, reasoning: "", answer: "" });
      publish();
      return;
    }
    const streaming = streamingRef.current.get(chatId);
    if ((type === "reasoning.delta" || type === "message.delta") && streaming) {
      const text = String(event.payload?.text ?? "");
      if (type === "reasoning.delta") streaming.reasoning += text;
      else streaming.answer += text;
      upsertStreaming(chatId, streaming);
      publish();
      return;
    }
    if (type === "message.complete") {
      streamingRef.current.delete(chatId);
      const messages = transcriptRef.current.get(chatId) ?? [];
      const text = String(event.payload?.text ?? "");
      const replyIndex = messages.findIndex((message) => message.status === "running" && message.kind !== "tool");
      const finalMessage = hermesMessage({
        role: "assistant",
        text,
        chatId,
        topicId: topicOf(chatId),
        id: `live:final:${turnKeyRef.current}`,
        at: Date.now() / 1000,
        agentName,
      });
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
  }, [agentName, chatForHandle, publish, refreshSessions, topicOf, upsertStreaming]);

  useEffect(() => {
    const lifetime = new AbortController();
    lifetimeRef.current = lifetime;
    const pending = new Map<number, {
      resolve: (value: unknown) => void;
      reject: (reason: Error) => void;
      timer: ReturnType<typeof setTimeout>;
    }>();
    let nextId = 1;
    let retryMs = 500;
    let retryTimer: ReturnType<typeof setTimeout> | undefined;

    const call: GatewayCall = (method, params = {}) => new Promise<unknown>((resolve, reject) => {
      const socket = socketRef.current;
      if (!socket || socket.readyState !== WebSocket.OPEN) {
        reject(new Error("Agent chat is not connected. Try again shortly."));
        return;
      }
      const id = nextId++;
      const timer = setTimeout(() => {
        pending.delete(id);
        reject(new Error("The agent did not answer in time. Check the conversation before retrying."));
      }, RPC_TIMEOUT_MS);
      pending.set(id, { resolve, reject, timer });
      try {
        socket.send(JSON.stringify({ jsonrpc: "2.0", id, method, params }));
      } catch (error) {
        clearTimeout(timer);
        pending.delete(id);
        reject(error instanceof Error ? error : new Error("Agent chat send failed."));
      }
    });
    callRef.current = call;

    const scheduleReconnect = (message: string) => {
      if (lifetime.signal.aborted) return;
      setStreamConnected(false);
      setTransportError(message);
      retryTimer = setTimeout(connect, retryMs);
      retryMs = Math.min(retryMs * 2, 10_000);
    };

    const connect = () => {
      if (lifetime.signal.aborted) return;
      void mintHostedHermesGatewaySocket(runtimeId, lifetime.signal)
        .then(({ url, protocols }) => {
          if (lifetime.signal.aborted) return;
          const socket = new WebSocket(url, protocols);
          socketRef.current = socket;
          socket.addEventListener("open", () => {
            if (socketRef.current !== socket) return;
            retryMs = 500;
            setStreamConnected(true);
            setTransportError(null);
          });
          socket.addEventListener("message", (event: MessageEvent) => {
            if (socketRef.current !== socket) return;
            let message: GatewayInbound;
            try {
              message = JSON.parse(String(event.data));
            } catch {
              return;
            }
            if (message?.method === "event" && message.params?.type) {
              handleEvent(message.params);
              return;
            }
            if (message?.id == null) return;
            const waiter = pending.get(message.id);
            if (!waiter) return;
            clearTimeout(waiter.timer);
            pending.delete(message.id);
            if (message.error) waiter.reject(new Error(message.error.message ?? "The agent refused that request."));
            else waiter.resolve(message.result);
          });
          socket.addEventListener("close", () => {
            if (socketRef.current !== socket) return;
            socketRef.current = null;
            // Gateway handles are per connection; stored sessions resume on demand.
            handlesRef.current.clear();
            streamingRef.current.clear();
            for (const [draftId, draft] of [...draftsRef.current]) {
              if (!draft.storedId) draftsRef.current.delete(draftId);
              else draft.handleId = "";
            }
            for (const waiter of pending.values()) {
              clearTimeout(waiter.timer);
              waiter.reject(new Error("Agent chat connection lost. Check the conversation before retrying."));
            }
            pending.clear();
            scheduleReconnect("Reconnecting to the agent…");
          });
          // Browsers emit close after error; reconnect is scheduled only there.
          socket.addEventListener("error", () => socket.close());
        })
        .catch((error: unknown) => {
          scheduleReconnect(error instanceof Error ? error.message : "Agent chat is unavailable.");
        });
    };

    connect();
    void refreshSessions()
      .then(() => undefined)
      .catch((error: unknown) => setTransportError(
        error instanceof Error ? error.message : "Could not load this agent's chats."
      ));

    return () => {
      lifetime.abort();
      if (retryTimer) clearTimeout(retryTimer);
      callRef.current = null;
      for (const waiter of pending.values()) {
        clearTimeout(waiter.timer);
        waiter.reject(new Error("Agent chat closed."));
      }
      pending.clear();
      const socket = socketRef.current;
      socketRef.current = null;
      socket?.close();
    };
  }, [handleEvent, refreshSessions, runtimeId]);

  const createDraft = useCallback(async () => {
    const call = callRef.current;
    if (!call) throw new Error("Agent chat is not connected. Try again shortly.");
    const created = (await call("session.create", { source: "desktop", cols: 100 })) as {
      session_id?: string;
      stored_session_id?: string;
    } | null;
    if (!created?.session_id) throw new Error("The agent did not start a chat. Try again.");
    const draftId = `draft:${created.session_id}`;
    draftsRef.current.set(draftId, {
      summary: {
        chat_id: draftId,
        title: "New chat",
        last_message_preview: "",
        unread_count: 0,
        message_count: 0,
        started_seq: Math.floor(Date.now() / 1000),
        updated_seq: Math.floor(Date.now() / 1000),
        active: true,
        archived: false,
      },
      handleId: created.session_id,
      storedId: created.stored_session_id ?? null,
    });
    transcriptRef.current.set(draftId, []);
    selectedRef.current = { topicId: HOME_TOPIC_ID, chatId: draftId };
    return draftId;
  }, []);

  const liveHandle = useCallback(async (chatId: string) => {
    const draft = draftsRef.current.get(chatId);
    if (draft) {
      if (!draft.handleId) throw new Error("This unsent chat lost its connection. Start a new chat.");
      return draft.handleId;
    }
    const existing = handlesRef.current.get(chatId);
    if (existing) return existing;
    const call = callRef.current;
    if (!call) throw new Error("Agent chat is not connected. Try again shortly.");
    const resumed = (await call("session.resume", { session_id: chatId })) as { session_id?: string } | null;
    if (!resumed?.session_id) throw new Error("The agent could not reopen this chat.");
    handlesRef.current.set(chatId, resumed.session_id);
    return resumed.session_id;
  }, []);

  const dispatch = useCallback(async (action: HostedChatAction): Promise<HostedChatState> => {
    if ("OpenChat" in action) {
      const { chat_id } = action.OpenChat;
      const sequence = ++selectionSequenceRef.current;
      selectedRef.current = { topicId: topicOf(chat_id), chatId: chat_id };
      publish();
      if (!draftsRef.current.has(chat_id)) {
        await loadTranscript(chat_id);
        if (sequence !== selectionSequenceRef.current) return currentState();
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
      // Platform topics are read-only history; every new web chat is native.
      await createDraft();
      publish();
      return currentState();
    }
    if ("SendChatMessage" in action || "SendTopicMessage" in action || "SendMessage" in action) {
      const text = "SendChatMessage" in action
        ? action.SendChatMessage.text
        : "SendTopicMessage" in action
          ? action.SendTopicMessage.text
          : action.SendMessage.text;
      let chatId = ("SendChatMessage" in action ? action.SendChatMessage.chat_id : null)
        ?? selectedRef.current.chatId;
      if (chatId && hermesSessionReadOnly(sessionFor(chatId) ?? { chatId: null })) {
        throw new Error(READ_ONLY_REASON);
      }
      if (!chatId) chatId = await createDraft();
      const handle = await liveHandle(chatId);
      const call = callRef.current;
      if (!call) throw new Error("Agent chat is not connected. Try again shortly.");
      const messages = transcriptRef.current.get(chatId) ?? [];
      const optimistic = hermesMessage({
        role: "user",
        text,
        chatId,
        topicId: HOME_TOPIC_ID,
        id: `local:${Date.now()}:${Math.random().toString(36).slice(2, 8)}`,
        at: Date.now() / 1000,
        agentName,
      });
      optimistic.outbound_delivery = { local_send: "Sending", server_delivery: "Undelivered" };
      messages.push(optimistic);
      transcriptRef.current.set(chatId, messages);
      publish();
      try {
        await call("prompt.submit", { session_id: handle, text });
        optimistic.outbound_delivery = { local_send: "Sent", server_delivery: "Delivered" };
      } catch (error) {
        const rows = transcriptRef.current.get(chatId) ?? [];
        const index = rows.findIndex((row) => row.message_id === optimistic.message_id);
        if (index >= 0) rows.splice(index, 1);
        publish();
        throw error;
      }
      publish();
      return currentState();
    }
    if ("RenameChat" in action) {
      const { chat_id, title } = action.RenameChat;
      if (hermesSessionReadOnly(sessionFor(chat_id) ?? { chatId: null })) throw new Error(READ_ONLY_REASON);
      const call = callRef.current;
      if (!call) throw new Error("Agent chat is not connected. Try again shortly.");
      await call("session.title", { session_id: await liveHandle(chat_id), title });
      await refreshSessions();
      return currentState();
    }
    // Read receipts and typing have no Hermes equivalent; publishing for them
    // would dispatch in a loop. Every other mutation fails visibly.
    if ("MarkRoomRead" in action || "SetTyping" in action) return currentState();
    throw new Error("This action is not available in Hermes chat yet.");
  }, [agentName, createDraft, currentState, liveHandle, loadTranscript, publish, refreshSessions, sessionFor, topicOf]);

  const load = useCallback(async (): Promise<HostedChatRetryAttempt> => {
    try {
      await refreshSessions();
      const chatId = selectedRef.current.chatId;
      if (chatId && !draftsRef.current.has(chatId)) {
        await loadTranscript(chatId);
        publish();
      }
      setTransportError(null);
      return "succeeded";
    } catch (error) {
      setTransportError(error instanceof Error ? error.message : "Could not load this agent's chats.");
      return "stop";
    }
  }, [loadTranscript, publish, refreshSessions]);

  const selectedChatId = state?.selected_chat_id ?? null;
  const selectedSession = selectedChatId ? sessionFor(selectedChatId) : null;
  const composerDisabledReason = selectedSession && hermesSessionReadOnly(selectedSession)
    ? READ_ONLY_REASON
    : null;

  const value = useMemo<HostedChatContextValue>(() => ({
    apiBase: `hermes:${runtimeId}`,
    canSendToTopic: true,
    supportsAttachments: false,
    supportsChatArchive: false,
    composerDisabledReason,
    state,
    transportError,
    claimError: null,
    sessionError: null,
    streamConnected,
    ownerClaimed: true,
    bindingRecoveryRequired: false,
    selectionPending: false,
    load,
    claimOwner: async () => "succeeded",
    recoverBinding: async () => "succeeded",
    reportSessionAuthFailure: () => false,
    signInAgain: () => window.location.reload(),
    noteComposerInput: () => undefined,
    dispatch,
    dispatchQuiet: async (action) => {
      try {
        return await dispatch(action);
      } catch {
        return null;
      }
    },
    refreshPendingChat: async (target: PendingChatRefreshTarget) => {
      if (streamingRef.current.has(target.chat_id) || draftsRef.current.has(target.chat_id)) return false;
      await loadTranscript(target.chat_id);
      publish();
      return true;
    },
    uploadAttachments: async () => {
      throw new Error("Attachments are not available in Hermes chat yet.");
    },
    attachmentUrl: () => "#",
  }), [composerDisabledReason, dispatch, load, loadTranscript, publish, runtimeId, state, streamConnected, transportError]);

  return <HostedChatContext.Provider value={value}>{children}</HostedChatContext.Provider>;
}

function emptyState(): HostedChatState {
  return {
    rev: 0,
    identity: { account_id: HERMES_VIEWER_ACCOUNT, device_id: "web" },
    rooms: [],
    topics: [],
    selected_room_id: null,
    selected_topic_id: null,
    selected_chat_id: null,
    active_profile_id: null,
    status: "ok",
    toast: null,
    messages: [],
    profiles: [],
    devices: [],
    typing_members: [],
    hosted_agent_binding: {
      version: 1,
      project_id: "hermes",
      human_account_id: HERMES_VIEWER_ACCOUNT,
      agent_account_id: HERMES_AGENT_ACCOUNT,
      agent_npub: "",
      canonical_room_id: HERMES_ROOM_ID,
      associated_room_ids: [HERMES_ROOM_ID],
    },
    flow: { notice_text: null, notice_busy: false, scan_in_flight: false, scan_result: "", image_upload_url: null },
  };
}

