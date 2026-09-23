"use client";

/** Native Hermes session transport for Finite's shared chat components.
 * Adapted from the existing Hermes gateway spike (PR #845). Core authorizes
 * each connection; Hermes owns sessions, history, and streamed turns. */
import { attemptHostedChatSignIn, currentHostedChatReturnPath, isHostedChatSessionAuthFailure, redirectToHostedChatSignIn, shouldAutoRedirectForSessionAuthFailure } from "@/lib/hosted-chat-session";
import { createHostedHermesWebSocket, readNativeRequesterContext, readHostedHermesJson, setHostedHermesSessionArchived } from "@/lib/hosted-hermes-status";
import { HOME_TOPIC_ID } from "@/lib/hosted-web-chat-topics";
import { hermesAttachmentUrl, hermesMessageAttachments } from "@/lib/hermes-attachments";
import { hermesBrainApprovals } from "@/lib/hermes-brain-approvals";
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

import { HermesApprovalCard, type HermesApproval } from "@/components/hermes-approval";

import { HermesClarificationCard, type HermesClarification } from "@/components/hermes-clarification";

const ROOM_ID = "hermes";
// hermes' own sidebar model: projects.tree claims sessions (scoped_session_ids)
// and everything left belongs in a flat Recents bucket. Rendered even when
// empty so New chat always has a home; new chats sent from Recents carry no
// cwd and hermes categorizes them on the first prompt.
// The shared New chat control requires Home; this is a UI projection only.
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
  archived?: boolean;
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

type Streaming = { turnKey: number; reasoning: string; answer: string; corrections?: { text: string; offset: number }[] };

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

export function HermesChatProvider({ children, machineId }: { children: ReactNode; machineId: string }) {
  const [clarifications, setClarifications] = useState<Record<string, HermesClarification | null>>({});
  const [approvals, setApprovals] = useState<Record<string, HermesApproval | null>>({});
  const [state, setState] = useState<HostedChatState | null>(null);
  const [transportError, setTransportError] = useState<string | null>(null);
  const [streamConnected, setStreamConnected] = useState(false);
  const [sessionError, setSessionError] = useState<string | null>(null);
  const composerInputRef = useRef({ draftText: "", attachmentCount: 0 });
  const reportSessionAuthFailure = useCallback((error: unknown) => {
    if (!isHostedChatSessionAuthFailure(error)) return false;
    setSessionError("Your session has ended. Sign in again to continue.");
    setTransportError(null);
    if (shouldAutoRedirectForSessionAuthFailure(composerInputRef.current)) {
      redirectToHostedChatSignIn(currentHostedChatReturnPath());
    }
    return true;
  }, []);
  const signInAgain = useCallback(() => {
    const error = attemptHostedChatSignIn(machineId, composerInputRef.current.draftText);
    if (error) setSessionError(error);
  }, [machineId]);
  const noteComposerInput = useCallback((input: { draftText: string; attachmentCount: number }) => {
    composerInputRef.current = input;
  }, []);

  const revRef = useRef(1);
  const seqRef = useRef(1);
  const chatsRef = useRef(new Map<string, ChatEntry>()); // keyed by chat_id
  const transcriptRef = useRef(new Map<string, HostedChatMessage[]>()); // chat_id → messages
  const projectsRef = useRef<GatewayProject[]>([]);
  const scopedRef = useRef<Set<string>>(new Set());
  const sessionsRef = useRef<GatewaySession[]>([]);
  const selectedRef = useRef<{ topicId: string | null; chatId: string | null }>({
    topicId: HOME_TOPIC_ID,
    chatId: null,
  });
  const turnKeyRef = useRef(0);
  const socketRef = useRef<WebSocket | null>(null);
  const requestAbortRef = useRef<AbortController | null>(null);
  const nextIdRef = useRef(1);
  const sendingRef = useRef(false);
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
        ? hermesBrainApprovals(transcriptRef.current.get(selected.chatId) ?? []).map((message) =>
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
  const upsertStreamingMessages = useCallback((chatId: string, topicId: string, streaming: Streaming, status: "running" | "complete" | "error" = "running") => {
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
    // Native offsets count Python characters, not JavaScript UTF-16 units.
    const characters = Array.from(streaming.answer);
    let start = 0;
    const corrections = streaming.corrections ?? [];
    for (let index = 0; index <= corrections.length; index++) {
      const correction = corrections[index];
      const end = correction ? Math.max(start, Math.min(correction.offset, characters.length)) : characters.length;
      if (end > start) upsert({
        ...gatewayMessage("assistant", characters.slice(start, end).join(""), chatId, topicId, false, status),
        message_id: `gateway:reply:${streaming.turnKey}:${index}`,
      });
      if (correction) upsert({
        ...gatewayMessage("user", correction.text, chatId, topicId, true),
        message_id: `gateway:correction:${streaming.turnKey}:${index}`,
      });
      start = end;
    }
    transcriptRef.current.set(chatId, messages);
  }, []);

  // Reopening and reconnecting bind a fresh transport to durable history.
  const hydrateTranscript = useCallback(async (chatId: string, topicId: string) => {
    const call = callRef.current;
    const selection = selectionSequenceRef.current;
    if (!call) return false;
    try {
      const resumed = (await call("session.resume", { session_id: chatId })) as {
        session_id: string;
        messages?: GatewayMessage[];
        running?: boolean;
        pending_approval?: HermesApproval;
        pending_clarify?: HermesClarification;
        inflight?: { status?: string; error?: string; assistant?: string; user?: string; streaming?: boolean; corrections?: string[]; correction_offsets?: number[] } | null;
      } | null;
      if (call !== callRef.current || selection !== selectionSequenceRef.current) return false;
      if (!resumed?.session_id) throw new Error("Gateway returned no session handle");
      setApprovals(current => ({ ...current, [chatId]: resumed.pending_approval ?? null }));
      setClarifications(current => ({ ...current, [chatId]: resumed.pending_clarify ?? null }));
      const rows = historyMessageRows(resumed.messages ?? [], chatId, topicId);
      setTransportError(null);
      const pendingUser = resumed.inflight?.user?.trim();
      const tail = [...rows].reverse().find(row => row.kind !== "tool");
      if (pendingUser && (!tail?.is_mine || tail.text !== hermesMessageAttachments(pendingUser).text)) {
        rows.push(gatewayMessage("user", pendingUser, chatId, topicId, true));
      }
      if (resumed.inflight?.status === "error") {
        const text = resumed.inflight.assistant || `Error: ${resumed.inflight.error || "Agent turn failed."}`;
        if (!resumed.inflight.corrections?.length) {
          if (rows.at(-1)?.sender_account_id !== AGENT_ACCOUNT || rows.at(-1)?.text !== text) rows.push(gatewayMessage("assistant", text, chatId, topicId, true, "error"));
          else rows[rows.length - 1].status = "error";
        }
        setTransportError(`Agent turn failed: ${resumed.inflight.error || "Unknown error"}`);
      }
      transcriptRef.current.set(chatId, rows);
      const entry = chatsRef.current.get(chatId);
      if (entry) {
        entry.handleId = resumed.session_id;
        const restored = { turnKey: ++turnKeyRef.current, reasoning: "", answer: resumed.inflight?.assistant ?? "",
          corrections: resumed.inflight?.corrections?.map((text, index) => ({ text,
            offset: resumed.inflight?.correction_offsets?.[index] ?? Array.from(resumed.inflight?.assistant ?? "").length })) };
        entry.streaming = resumed.running && resumed.inflight?.status !== "error" ? restored : null;
        if (entry.streaming) upsertStreamingMessages(chatId, topicId, entry.streaming);
        else if (resumed.inflight?.status === "error" && restored.corrections?.length) upsertStreamingMessages(chatId, topicId, restored, "error");
      }
      publish();
    } catch {
      if (call !== callRef.current || selection !== selectionSequenceRef.current) return false;
      setTransportError("Could not reload this conversation. Retry before sending.");
      return false;
    }
    return true;
  }, [publish, upsertStreamingMessages]);

  const refreshLists = useCallback(async () => {
    const call = callRef.current;
    const signal = requestAbortRef.current?.signal;
    if (!call || !signal) return;
    const sequence = ++listSequenceRef.current;
    const [treeResult, listResult] = await Promise.all([
      call("projects.tree", { preview_limit: 50 }),
      (async () => {
        const sessions = new Map<string, GatewaySession>();
        // Hermes caps each page at 100. Walk its inventory before publishing so
        // a failed later page cannot replace the sidebar with partial history.
        for (let offset = 0; ; offset += 100) {
          signal.throwIfAborted();
          const page = await readHostedHermesJson(machineId,
            `api/sessions?limit=100&archived=include&order=created&offset=${offset}`, signal) as { sessions: GatewaySession[] };
          const before = sessions.size;
          for (const session of page.sessions) sessions.set(session.id, session);
          if (page.sessions.length < 100) return { sessions: [...sessions.values()] };
          if (sessions.size === before) throw new Error("Agent session pagination did not advance.");
        }
      })(),
    ]);
    if (sequence !== listSequenceRef.current || call !== callRef.current) return;
    const tree = treeResult as { projects?: GatewayProject[]; scoped_session_ids?: string[] } | null;
    const list = listResult as { sessions?: GatewaySession[] } | null;
    projectsRef.current = (tree?.projects ?? []).filter((project) => project.sessionCount > 0);
    scopedRef.current = new Set(tree?.scoped_session_ids ?? []);
    const sessions = (list?.sessions ?? []).filter((session) => !["tool", "kanban"].includes(session.source));
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
  }, [machineId, publish]);

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
      if (type === "clarify.request") {
        const request = event.payload as HermesClarification | undefined;
        if (request?.request_id) setClarifications(current => ({ ...current, [chatId]: request }));
      } else if (type === "clarify.expire") {
        setClarifications(current => current[chatId]?.request_id === event.payload?.request_id
          ? { ...current, [chatId]: null } : current);
      } else if (type === "approval.request") {
        const request = event.payload as HermesApproval | undefined;
        if (request?.request_id && typeof request.command === "string" && Array.isArray(request.choices)) {
          setApprovals(current => ({ ...current, [chatId]: request }));
        }
      } else if (type === "tool.start" || type === "tool.complete") {
        const payload = event.payload;
        if (typeof payload?.tool_id !== "string" || !payload.tool_id) return;
        const messages = transcriptRef.current.get(chatId) ?? [];
        const result = payload.result_text ?? payload.result;
        const text = type === "tool.complete" && result !== undefined
          ? typeof result === "string" ? result : JSON.stringify(result)
          : [payload.name, payload.context].filter(Boolean).join(": ");
        const message = {
          ...gatewayMessage("assistant", text, chatId, chat.topicId, false, type === "tool.start" ? "running" : "complete"),
          message_id: `${chatId}:tool:${payload.tool_id}`,
          kind: "tool",
        };
        const index = messages.findIndex(row => row.message_id === message.message_id);
        if (index >= 0) messages.splice(index, 1, message);
        else {
          // The native reply accumulates in one row. Keep tool output before
          // that row, so final-delivery cards also see references from late tools.
          const reply = messages.findIndex(row => row.status === "running" && row.kind === "message" && !row.is_mine);
          messages.splice(reply < 0 ? messages.length : reply, 0, message);
        }
        transcriptRef.current.set(chatId, messages);
        publish();
      } else if (type === "message.start") {
        if (selectedRef.current.chatId === chatId) setTransportError(null);
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
        setApprovals(current => ({ ...current, [chatId]: null }));
        setClarifications(current => ({ ...current, [chatId]: null }));
        const text = String(event.payload?.text ?? "");
        const corrected = chat.streaming?.corrections?.length;
        if (corrected && chat.streaming) {
          chat.streaming.answer = text;
          upsertStreamingMessages(chatId, chat.topicId, chat.streaming, event.payload?.status === "error" ? "error" : "complete");
        }
        chat.streaming = null;
        const messages = transcriptRef.current.get(chatId) ?? [];
        // The reply row is replaced by the final text; every live rollup row
        // (the thinking trace) settles to complete.
        const replyIndex = messages.findIndex(
          (message) => message.status === "running" && message.kind !== "tool"
        );
        const failed = event.payload?.status === "error";
        if (failed && selectedRef.current.chatId === chatId) setTransportError(`Agent turn failed: ${String(event.payload?.error || "Unknown error")}`);
        const finalMessage = gatewayMessage("assistant", text, chatId, chat.topicId, false, failed ? "error" : "complete");
        if (!corrected) {
          if (replyIndex >= 0) messages.splice(replyIndex, 1, finalMessage);
          else if (text) messages.push(finalMessage);
        }
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

    const connectionAbort = new AbortController();
    requestAbortRef.current = connectionAbort;
    const connect = () => {
      if (disposed) return;
      void createHostedHermesWebSocket(machineId, connectionAbort.signal)
        .then((target) => {
          if (disposed) return;
          openSocket(target);
        })
        .catch((error: unknown) => {
          if (disposed) return;
          setStreamConnected(false);
          if (reportSessionAuthFailure(error)) return;
          setTransportError("Agent chat is unavailable. Reconnecting…");
          retryTimer = setTimeout(connect, retryMs);
          retryMs = Math.min(retryMs * 2, 10_000);
        });
    };

    const openSocket = (target: { url: string; protocols: string[] }) => {
      if (disposed) return;
      const socket = new WebSocket(target.url, target.protocols);
      socketRef.current = socket;

      socket.addEventListener("open", () => {
        if (disposed || socketRef.current !== socket) return;
        retryMs = 500;
        setStreamConnected(true);
        setTransportError(null);
        void refreshLists().then(async () => {
          const selected = selectedRef.current;
          if (selected.chatId && !selected.chatId.startsWith("draft:")) {
            await hydrateTranscript(selected.chatId, selected.topicId ?? HOME_TOPIC_ID);
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
        publish();
        setStreamConnected(false);
        setTransportError("Agent chat is unavailable. Reconnecting…");
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
      connectionAbort.abort();
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
  }, [handleEvent, hydrateTranscript, machineId, publish, refreshLists, reportSessionAuthFailure]);

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

  const sendMessage = useCallback(async (chatId: string | null, topicId: string, text: string, files: File[] = []) => {
    if (sendingRef.current) throw new Error("A message is already being sent.");
    if (files.length > 8 || files.some(file => file.size > 32 * 1024 * 1024) || files.reduce((total, file) => total + file.size, 0) > 64 * 1024 * 1024) throw new Error("Attachments exceed the upload limit.");
    const call = callRef.current;
    const socket = socketRef.current;
    const assertConnected = () => {
      if (!call || call !== callRef.current || !socket || socket !== socketRef.current || socket.readyState !== WebSocket.OPEN) throw new Error("Agent connection changed. Reopen the conversation before sending.");
    };
    assertConnected();
    if (!call) throw new Error("Agent chat is unavailable.");
    if (chatId && !chatsRef.current.has(chatId)) throw new Error("Conversation is unavailable. Reopen it before sending.");
    sendingRef.current = true;
    try {
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
      assertConnected();
      const handle = entry.handleId;
      const references: string[] = [];
      for (const file of files) {
        const dataUrl = await new Promise<string>((resolve, reject) => {
          const reader = new FileReader();
          reader.onload = () => resolve(String(reader.result));
          reader.onerror = () => reject(new Error(`Could not read ${file.name}.`));
          reader.readAsDataURL(file);
        });
        assertConnected();
        const uploaded = await call("file.attach", { session_id: handle, name: file.name, data_url: dataUrl }) as { attached?: boolean; path?: string };
        if (!uploaded.attached || !uploaded.path?.startsWith("/") || /[\x00-\x1f]/.test(uploaded.path)) throw new Error("Agent returned an invalid attachment.");
        const quote = ["`", '\"', "'"].find(char => !uploaded.path!.includes(char));
        if (!quote) throw new Error("Attachment filename is unsupported.");
        // Bind references to this turn; Hermes owns vision tool execution.
        references.push(`@${file.type.startsWith("image/") ? "image" : "file"}:${quote}${uploaded.path}${quote}`);
      }
      assertConnected();
      text = [text, ...references].filter(Boolean).join("\n");
      const messages = transcriptRef.current.get(chatId) ?? [];
      const optimistic = gatewayMessage("user", text, chatId, entry.topicId, false);
      optimistic.outbound_delivery = { local_send: "Sending", server_delivery: "Undelivered" };
      messages.push(optimistic);
      transcriptRef.current.set(chatId, messages);
      // Publish the optimistic user message BEFORE awaiting the turn so a
      // slow gateway can never freeze the composer.
      publish();
      try {
        const streaming = entry.streaming;
        const offset = Array.from(streaming?.answer ?? "").length;
        const signal = requestAbortRef.current?.signal;
        if (!signal) throw new Error("Agent connection changed. Reopen the conversation before sending.");
        const requester = await readNativeRequesterContext(machineId, signal);
        assertConnected();
        const result = await call("prompt.submit", { session_id: handle, text, ...(requester ? { finite_requester: requester } : {}) }) as { status?: string };
        if (streaming && streaming === entry.streaming && (result?.status === "redirected" || result?.status === "steered")) {
          const index = messages.findIndex(row => row.message_id === optimistic.message_id);
          if (index >= 0) messages.splice(index, 1);
          (streaming.corrections ??= []).push({ text, offset });
          upsertStreamingMessages(chatId, topicId, streaming);
        }
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
    } finally {
      sendingRef.current = false;
    }
  }, [machineId, createDraft, currentState, publish, upsertStreamingMessages]);

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
          const loaded = await hydrateTranscript(chat_id, entry.topicId);
          if (sequence !== selectionSequenceRef.current) return currentState();
          if (!loaded) throw new Error("Could not reload this conversation. Retry before sending.");
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
        const chatId = explicitChatId ?? selectedRef.current.chatId;
        const topicId = chatScoped?.topic_id
          ?? ("SendTopicMessage" in action ? action.SendTopicMessage.topic_id : selectedRef.current.topicId)
          ?? HOME_TOPIC_ID;
        return sendMessage(chatId, topicId, text);
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

      if ("SetChatArchived" in action) {
        const { chat_id, archived } = action.SetChatArchived;
        const entry = chatsRef.current.get(chat_id);
        const signal = requestAbortRef.current?.signal;
        if (!entry || !signal) throw new Error("Conversation is unavailable. Reopen it before archiving.");
        const storedId = entry.storedId ?? chat_id;
        await setHostedHermesSessionArchived(machineId, storedId, archived, signal);
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
    [createDraft, currentState, hydrateTranscript, machineId, publish, refreshLists, sendMessage]
  );

  const load = useCallback(async (): Promise<HostedChatRetryAttempt> => {
    try {
      await refreshLists();
      setTransportError(null);
      const selected = selectedRef.current;
      if (selected.chatId && !selected.chatId.startsWith("draft:")) {
        const loaded = await hydrateTranscript(selected.chatId, selected.topicId ?? HOME_TOPIC_ID);
        if (!loaded) return "stop";
      }
      return "succeeded";
    } catch {
      setTransportError("Agent chat is unavailable. Reconnecting…");
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
  const uploadAttachments = useCallback(async (form: FormData): Promise<HostedChatState> => {
    const files = form.getAll("files").filter((file): file is File => file instanceof File);
    if (!files.length) throw new Error("Choose a file to attach.");
    const chatId = form.get("chat_id");
    const topicId = form.get("topic_id");
    const caption = form.get("caption");
    return sendMessage(typeof chatId === "string" ? chatId : selectedRef.current.chatId,
      typeof topicId === "string" ? topicId : HOME_TOPIC_ID,
      typeof caption === "string" ? caption : "", files);
  }, [sendMessage]);

  const interruptTurn = useCallback(async () => {
    const selected = selectedRef.current.chatId;
    const entry = selected ? chatsRef.current.get(selected) : undefined;
    const call = callRef.current;
    if (!call || !entry?.handleId || !entry.streaming) throw new Error("No connected agent turn to stop.");
    // The acknowledgement requests cancellation. Only Hermes's terminal event
    // settles the turn; do not invent completion or retry an ambiguous request.
    await call("session.interrupt", { session_id: entry.handleId });
  }, []);

  const respondApproval = useCallback(async (chatId: string, request: HermesApproval, choice: "once" | "deny") => {
    const call = callRef.current;
    const entry = chatsRef.current.get(chatId);
    if (!call || !entry?.handleId) throw new Error("Reconnect before answering this request.");
    const result = await call("approval.respond", {
      session_id: entry.handleId, request_id: request.request_id, choice,
    }) as { resolved?: number };
    // Never approve all pending requests or replay an uncertain response.
    if (call !== callRef.current) return;
    setApprovals(current => current[chatId]?.request_id === request.request_id
      ? { ...current, [chatId]: null } : current);
    // Hermes owns the queue. Reload its next pending request instead of
    // hiding an older approval behind the most recently received event.
    if (selectedRef.current.chatId === chatId) await hydrateTranscript(chatId, entry.topicId);
    if (!result.resolved) setTransportError("This approval expired or was already answered.");
  }, [hydrateTranscript]);
  const respondClarification = useCallback(async (chatId: string, request: HermesClarification, answer: string, questionId?: string) => {
    const call = callRef.current;
    const entry = chatsRef.current.get(chatId);
    if (!call || !entry?.handleId) throw new Error("Reconnect before answering this question.");
    const result = await call("clarify.respond", {
      session_id: entry.handleId, request_id: request.request_id, answer,
      ...(questionId ? { question_id: questionId } : {}),
    }) as { status?: string; remaining?: string[] };
    if (call !== callRef.current) return;
    setClarifications(current => current[chatId]?.request_id !== request.request_id ? current : {
      ...current, [chatId]: result.remaining?.length && questionId
        ? { ...current[chatId]!, answers: { ...current[chatId]?.answers, [questionId]: answer } } : null,
    });
    if (result.status === "expired") setTransportError("This question has expired. Ask the agent to continue.");
  }, []);
  const actionChatId = state?.selected_chat_id;
  const approval = actionChatId ? approvals[actionChatId] : null;
  const clarification = actionChatId ? clarifications[actionChatId] : null;
  const pendingAgentAction = useMemo(() => actionChatId ? <>
    {approval && <HermesApprovalCard key={`${actionChatId}:${approval.request_id}`} request={approval} connected={streamConnected}
      respond={choice => respondApproval(actionChatId, approval, choice)} />}
    {clarification && <HermesClarificationCard key={`${actionChatId}:${clarification.request_id}`} request={clarification} connected={streamConnected}
      respond={(answer, questionId) => respondClarification(actionChatId, clarification, answer, questionId)} />}
  </> : undefined, [approval, clarification, actionChatId, respondApproval, respondClarification, streamConnected]);

  const value = useMemo<HostedChatContextValue>(
    () => ({
      apiBase: "hermes-gateway",
      pendingAgentAction,
      state,
      transportError,
      sessionError,
      reportSessionAuthFailure,
      signInAgain,
      noteComposerInput,
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
      interruptTurn: state?.typing_members.length ? interruptTurn : undefined,
      attachmentUrl: ({ attachment_id }) => hermesAttachmentUrl(machineId, attachment_id),
    }),
    [claimOwner, dispatch, dispatchQuiet, interruptTurn, load, machineId, recoverBinding, refreshPendingChat, state, streamConnected, transportError, uploadAttachments, sessionError, reportSessionAuthFailure, signInAgain, noteComposerInput, pendingAgentAction]
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
    archived: session.archived ?? false,
  };
}

function topicIdForProject(project: GatewayProject): string {
  return `project:${project.id}`;
}

function topicOfSession(session: GatewaySession, projects: GatewayProject[]): string {
  const project = projects.find((candidate) =>
    candidate.previewSessions.some((candidateSession) => candidateSession.id === session.id)
  );
  return project ? topicIdForProject(project) : HOME_TOPIC_ID;
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
      .filter((entry) => entry.topicId === HOME_TOPIC_ID)
      .map((entry) => entry.summary),
    sessions.filter((session) => !scoped.has(session.id)).map(summaryFromSession)
  );
  topics.push({
    room_id: ROOM_ID,
    topic_id: HOME_TOPIC_ID,
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
  status: "running" | "complete" | "error" = "complete"
): HostedChatMessage {
  const mine = role === "user";
  const content = hermesMessageAttachments(text);
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
    text: content.text,
    display_content: content.text,
    rich_text_json: undefined,
    metadata_json: undefined,
    reply_to_message_id: null,
    is_mine: mine,
    outbound_delivery: mine ? { local_send: "Sent", server_delivery: "Delivered" } : null,
    media: content.media,
    kind: "message",
    status,
    final_delivery: status !== "running" || historical,
    edit_of_message_id: null,
    timestamp_unix_seconds: seconds,
    display_timestamp: new Date(seconds * 1000).toLocaleTimeString([], {
      hour: "numeric",
      minute: "2-digit",
    }),
    ...(historical ? {} : {}),
  };
}
