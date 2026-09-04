"use client";

/**
 * Spike: web chat over the hermes-agent tui_gateway, DIRECT from the browser.
 *
 * This file is the entire new-world frontend, deliberately self-contained so
 * the future cutover (deleting the finitechat hosted-device chat infra) is a
 * file-shaped operation. No Next API routes, no server-side client, no cache:
 * one WebSocket to the gateway, JSON-RPC in, events out, state rendered.
 *
 * Connection info is inlined by Next from public env (dev-local values):
 *   NEXT_PUBLIC_HERMES_GATEWAY_WS_URL   ws://127.0.0.1:9119/api/ws
 *   NEXT_PUBLIC_HERMES_GATEWAY_TOKEN    HERMES_DASHBOARD_SESSION_TOKEN the
 *                                       `hermes serve` process was launched
 *                                       with (rides ?token=). In a real
 *                                       deployment the per-viewer token is
 *                                       the ONE injection point to change.
 *
 * Verified protocol contract (hermes v2026.8.3, captured live):
 * - newline-delimited JSON-RPC; `gateway.ready` on connect.
 * - Lists pull, turns push. `sessions.changed` (no session_id) is the
 *   broadcast that the session list moved — refetch on it.
 * - Sidebar: `projects.tree` (projects ≈ topics, by session cwd) +
 *   `session.list`; sessions claimed by a project are excluded from flat
 *   Recents by `scoped_session_ids`.
 * - Opening a stored session: `session.resume {session_id}` returns a FRESH
 *   short gateway id plus the transcript inline (cold `session.history`
 *   without resume is "session not found").
 * - New chat: `session.create` returns a draft id; drafts have NO DB row and
 *   are invisible to lists until the first `prompt.submit`, and get reaped
 *   (`session.reclaimed` / ws_orphan_reap) if their connection dies first.
 * - A turn: `message.start` → `thinking.delta` (indicator) → `reasoning.delta`
 *   (stream) → `message.delta` (answer stream) → `message.complete`
 *   {text, usage, status, reasoning}. `prompt.submit` acks with
 *   {"status":"streaming"} immediately.
 */

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { ChevronRightIcon, HashIcon, PlusIcon, RotateCcwIcon, XIcon } from "lucide-react";

const GATEWAY_WS_URL =
  process.env.NEXT_PUBLIC_HERMES_GATEWAY_WS_URL || "ws://127.0.0.1:9119/api/ws";
const GATEWAY_TOKEN = process.env.NEXT_PUBLIC_HERMES_GATEWAY_TOKEN || "";

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

type GatewayTree = {
  projects: GatewayProject[];
  scoped_session_ids: string[];
};

type GatewayMessage = { role: string; text: string };

type ActiveChat = {
  /** Short gateway id used for RPCs (draft id, or the one session.resume returned). */
  handleId: string;
  /** Stored id shown in lists; null while the chat is still an unprompted draft. */
  storedId: string | null;
  title: string;
  transcript: GatewayMessage[];
};

type Streaming = { reasoning: string; answer: string };

/** One WebSocket, JSON-RPC correlation by id, event dispatch, auto-reconnect. */
function useHermesGateway(onEvent: (event: GatewayEvent) => void) {
  const [connection, setConnection] = useState<"connecting" | "ready" | "error">("connecting");
  const [connectionError, setConnectionError] = useState<string | null>(null);
  const [tree, setTree] = useState<GatewayTree>({ projects: [], scoped_session_ids: [] });
  const [sessions, setSessions] = useState<GatewaySession[]>([]);

  const socketRef = useRef<WebSocket | null>(null);
  const nextIdRef = useRef(1);
  const pendingRef = useRef(
    new Map<number, { resolve: (value: unknown) => void; reject: (reason: Error) => void }>()
  );
  const onEventRef = useRef(onEvent);
  onEventRef.current = onEvent;

  const call = useCallback(
    (method: string, params: Record<string, unknown> = {}) =>
      new Promise<unknown>((resolve, reject) => {
        const socket = socketRef.current;
        if (socket == null || socket.readyState !== WebSocket.OPEN) {
          reject(new Error("gateway socket is not open"));
          return;
        }
        const id = nextIdRef.current++;
        pendingRef.current.set(id, { resolve, reject });
        socket.send(JSON.stringify({ jsonrpc: "2.0", id, method, params }));
      }),
    []
  );

  const refresh = useCallback(async () => {
    const [treeResult, listResult] = await Promise.all([
      call("projects.tree", { preview_limit: 25 }),
      call("session.list", { limit: 100 }),
    ]);
    const treePayload = treeResult as GatewayTree | null;
    const listPayload = listResult as { sessions?: GatewaySession[] } | null;
    setTree({
      projects: treePayload?.projects ?? [],
      scoped_session_ids: treePayload?.scoped_session_ids ?? [],
    });
    setSessions(listPayload?.sessions ?? []);
  }, [call]);

  const refreshRef = useRef(refresh);
  refreshRef.current = refresh;

  useEffect(() => {
    let disposed = false;
    let retryMs = 500;
    let retryTimer: ReturnType<typeof setTimeout> | undefined;

    const connect = () => {
      if (disposed) return;
      setConnection("connecting");
      const target = GATEWAY_TOKEN
        ? `${GATEWAY_WS_URL}?token=${encodeURIComponent(GATEWAY_TOKEN)}`
        : GATEWAY_WS_URL;
      const socket = new WebSocket(target);
      socketRef.current = socket;

      socket.addEventListener("open", () => {
        retryMs = 500;
        setConnection("ready");
        setConnectionError(null);
        void refreshRef.current().catch(() => undefined);
      });

      socket.addEventListener("message", (event: MessageEvent) => {
        let message: GatewayInbound;
        try {
          message = JSON.parse(String(event.data));
        } catch {
          return;
        }
        if (message.method === "event" && message.params?.type) {
          // The list-moved broadcast is a list concern: refetch right here so
          // event consumers only ever see turn activity.
          if (message.params.type === "sessions.changed") {
            void refreshRef.current().catch(() => undefined);
            return;
          }
          onEventRef.current(message.params);
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
          pending.reject(new Error("gateway connection lost"));
        }
        pendingRef.current.clear();
        setConnection("error");
        setConnectionError(`gateway unreachable at ${GATEWAY_WS_URL}`);
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
  }, []);

  return { connection, connectionError, tree, sessions, call, refresh };
}

/**
 * The hermes half of the parity probe: sidebar section (projects ≈ topics,
 * then flat Recents) plus a portal-rendered chat pane — new blank chat, send,
 * stream, and watch the session get categorized once it says something.
 */
export function HermesGatewayChat() {
  const [active, setActive] = useState<ActiveChat | null>(null);
  const [streaming, setStreaming] = useState<Streaming | null>(null);
  const [draft, setDraft] = useState("");
  const [busy, setBusy] = useState(false);
  const [chatError, setChatError] = useState<string | null>(null);

  const activeRef = useRef<ActiveChat | null>(null);
  activeRef.current = active;

  const handleEvent = useCallback((event: GatewayEvent) => {
    const activeChat = activeRef.current;
    if (!activeChat || event.session_id !== activeChat.handleId) return;
    const type = event.type ?? "";
    if (type === "message.start") {
      setStreaming({ reasoning: "", answer: "" });
    } else if (type === "reasoning.delta") {
      setStreaming((current) =>
        current ? { ...current, reasoning: current.reasoning + String(event.payload?.text ?? "") } : current
      );
    } else if (type === "message.delta") {
      setStreaming((current) =>
        current ? { ...current, answer: current.answer + String(event.payload?.text ?? "") } : current
      );
    } else if (type === "message.complete") {
      const text = String(event.payload?.text ?? "");
      setActive((current) =>
        current
          ? { ...current, transcript: [...current.transcript, { role: "assistant", text }] }
          : current
      );
      setStreaming(null);
    }
  }, []);

  const gateway = useHermesGateway(handleEvent);

  const recents = useMemo(
    () => gateway.sessions.filter((session) => !gateway.tree.scoped_session_ids.includes(session.id)),
    [gateway.sessions, gateway.tree.scoped_session_ids]
  );
  const projects = useMemo(
    () => gateway.tree.projects.filter((project) => project.sessionCount > 0),
    [gateway.tree.projects]
  );

  async function openSession(session: GatewaySession) {
    setBusy(true);
    setChatError(null);
    setStreaming(null);
    try {
      const result = (await gateway.call("session.resume", { session_id: session.id })) as {
        session_id: string;
        messages?: GatewayMessage[];
      } | null;
      if (!result?.session_id) throw new Error("session.resume returned no handle");
      setActive({
        handleId: result.session_id,
        storedId: session.id,
        title: session.title || session.preview || "Untitled session",
        transcript: result.messages ?? [],
      });
    } catch (caught) {
      setChatError(caught instanceof Error ? caught.message : "could not open session");
    } finally {
      setBusy(false);
    }
  }

  async function newChat() {
    setBusy(true);
    setChatError(null);
    setStreaming(null);
    try {
      const result = (await gateway.call("session.create", { source: "gateway", cols: 100 })) as {
        session_id: string;
      } | null;
      if (!result?.session_id) throw new Error("session.create returned no draft id");
      setActive({
        handleId: result.session_id,
        storedId: null,
        title: "New chat",
        transcript: [],
      });
    } catch (caught) {
      setChatError(caught instanceof Error ? caught.message : "could not create chat");
    } finally {
      setBusy(false);
    }
  }

  async function send() {
    const text = draft.trim();
    if (!text || !active || busy) return;
    setBusy(true);
    setChatError(null);
    setDraft("");
    try {
      await gateway.call("prompt.submit", { session_id: active.handleId, text });
      setActive((current) =>
        current ? { ...current, transcript: [...current.transcript, { role: "user", text }] } : current
      );
      setStreaming({ reasoning: "", answer: "" });
    } catch (caught) {
      setChatError(caught instanceof Error ? caught.message : "could not send");
      setDraft(text);
    } finally {
      setBusy(false);
    }
  }

  return (
    <>
      <SidebarSection
        connection={gateway.connection}
        connectionError={gateway.connectionError}
        projects={projects}
        recents={recents}
        activeStoredId={active?.storedId ?? null}
        busy={busy}
        onRefresh={() => void gateway.refresh().catch(() => undefined)}
        onNewChat={() => void newChat()}
        onOpen={(session) => void openSession(session)}
        error={chatError}
      />
      {active ? createPortal(<ChatPane
        active={active}
        streaming={streaming}
        draft={draft}
        busy={busy}
        error={chatError}
        onDraftChange={setDraft}
        onSend={() => void send()}
        onClose={() => {
          setActive(null);
          setStreaming(null);
        }}
      />, document.body) : null}
    </>
  );
}

function SidebarSection({
  connection,
  connectionError,
  projects,
  recents,
  activeStoredId,
  busy,
  onRefresh,
  onNewChat,
  onOpen,
  error,
}: {
  connection: "connecting" | "ready" | "error";
  connectionError: string | null;
  projects: GatewayProject[];
  recents: GatewaySession[];
  activeStoredId: string | null;
  busy: boolean;
  onRefresh: () => void;
  onNewChat: () => void;
  onOpen: (session: GatewaySession) => void;
  error: string | null;
}) {
  const [collapsed, setCollapsed] = useState<Set<string>>(() => new Set());
  const toggle = (key: string) =>
    setCollapsed((current) => {
      const next = new Set(current);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  const rowTitle = (session: GatewaySession) =>
    session.title || session.preview || "Untitled session";

  return (
    <>
      <div className="finite-chat__sidebar-section-row">
        <span className="finite-chat__sidebar-section">Hermes · api/ws</span>
        <button
          type="button"
          className="ocean-icon-button"
          aria-label="New hermes chat"
          title="New hermes chat"
          disabled={busy}
          onClick={onNewChat}
        >
          <PlusIcon className="size-3.5" />
        </button>
        <button
          type="button"
          className="ocean-icon-button"
          aria-label="Reload hermes sessions"
          title="Reload hermes sessions"
          onClick={onRefresh}
        >
          <RotateCcwIcon className="size-3.5" />
        </button>
      </div>
      {connection === "connecting" ? (
        <p className="finite-agent-sidebar__status">Connecting to hermes gateway…</p>
      ) : null}
      {connection === "error" ? (
        <div className="finite-agent-sidebar__error">
          <span>{connectionError}</span>
        </div>
      ) : null}
      {error ? (
        <div className="finite-agent-sidebar__error">
          <span>{error}</span>
        </div>
      ) : null}
      {connection === "ready" ? (
        <>
          {projects.map((project) => {
            const isCollapsed = collapsed.has(project.id);
            return (
              <div className="finite-chat__folder" key={project.id}>
                <div className="finite-chat__folder-header">
                  <button
                    type="button"
                    className="finite-chat__folder-summary"
                    aria-expanded={!isCollapsed}
                    aria-label={`${isCollapsed ? "Expand" : "Collapse"} ${project.label}`}
                    onClick={() => toggle(project.id)}
                  >
                    <span className="finite-chat__folder-main">
                      <span className="finite-chat__folder-icon" aria-hidden>
                        <HashIcon className="size-3.5" />
                      </span>
                      <span className="finite-chat__folder-label">{project.label}</span>
                    </span>
                    {project.sessionCount > 0 ? (
                      <span className="finite-chat__unread-count">{project.sessionCount}</span>
                    ) : null}
                    <ChevronRightIcon className="finite-chat__topic-collapse-icon size-3.5" aria-hidden />
                  </button>
                </div>
                <div className="finite-chat__folder-body" hidden={isCollapsed}>
                  {project.previewSessions.map((session) => (
                    <GatewaySessionRow
                      key={session.id}
                      session={session}
                      title={rowTitle(session)}
                      active={session.id === activeStoredId}
                      onOpen={() => onOpen(session)}
                    />
                  ))}
                </div>
              </div>
            );
          })}
          <div className="finite-chat__folder">
            <div className="finite-chat__folder-header">
              <button
                type="button"
                className="finite-chat__folder-summary"
                aria-expanded={!collapsed.has("__recents")}
                onClick={() => toggle("__recents")}
              >
                <span className="finite-chat__folder-main">
                  <span className="finite-chat__folder-label">Recents</span>
                </span>
                <ChevronRightIcon className="finite-chat__topic-collapse-icon size-3.5" aria-hidden />
              </button>
            </div>
            <div className="finite-chat__folder-body" hidden={collapsed.has("__recents")}>
              {recents.map((session) => (
                <GatewaySessionRow
                  key={session.id}
                  session={session}
                  title={rowTitle(session)}
                  active={session.id === activeStoredId}
                  onOpen={() => onOpen(session)}
                />
              ))}
              {recents.length === 0 ? (
                <p className="finite-agent-sidebar__status">No unprojected sessions.</p>
              ) : null}
            </div>
          </div>
        </>
      ) : null}
    </>
  );
}

function GatewaySessionRow({
  session,
  title,
  active,
  onOpen,
}: {
  session: GatewaySession;
  title: string;
  active: boolean;
  onOpen: () => void;
}) {
  return (
    <div className={`finite-chat__thread-row ${active ? "is-active" : ""}`}>
      <button
        type="button"
        className="finite-chat__thread-open"
        aria-current={active ? "page" : undefined}
        title={`${session.preview}\n${session.message_count} messages · ${session.source || "unknown source"} · via tui_gateway`}
        onClick={onOpen}
      >
        <span className="finite-chat__thread-indicator" aria-hidden />
        <span className="finite-chat__thread-main">
          <span className="finite-chat__thread-title">{title}</span>
        </span>
      </button>
    </div>
  );
}

/**
 * Portal overlay standing in for the main chat pane, so no existing chat
 * component is touched. Bare Tailwind on purpose: this pane is throwaway
 * scaffolding until the cutover makes it the real pane.
 */
function ChatPane({
  active,
  streaming,
  draft,
  busy,
  error,
  onDraftChange,
  onSend,
  onClose,
}: {
  active: ActiveChat;
  streaming: Streaming | null;
  draft: string;
  busy: boolean;
  error: string | null;
  onDraftChange: (value: string) => void;
  onSend: () => void;
  onClose: () => void;
}) {
  const bottomRef = useRef<HTMLDivElement | null>(null);
  useEffect(() => {
    bottomRef.current?.scrollIntoView({ block: "end" });
  }, [active.transcript.length, streaming?.answer, streaming?.reasoning]);

  return (
    <div className="fixed inset-0 left-[224px] z-50 flex flex-col bg-[#0b0e14] text-[#e7e9ee]">
      <header className="flex items-center gap-3 border-b border-white/10 px-5 py-3">
        <span className="text-sm font-medium">{active.title}</span>
        <span className="rounded bg-white/10 px-2 py-0.5 text-[11px] uppercase tracking-wide text-white/60">
          hermes · tui_gateway
        </span>
        {!active.storedId ? (
          <span className="text-[11px] text-amber-300/80">
            draft — invisible to the sidebar until it sends its first message
          </span>
        ) : null}
        <button
          type="button"
          className="ml-auto rounded p-1 text-white/60 hover:bg-white/10"
          aria-label="Close hermes chat"
          onClick={onClose}
        >
          <XIcon className="size-4" />
        </button>
      </header>
      <div className="flex-1 space-y-4 overflow-y-auto px-5 py-4">
        {active.transcript.map((message, index) => (
          <div
            key={index}
            className={
              message.role === "user"
                ? "ml-auto max-w-[70%] rounded-2xl bg-blue-600/80 px-4 py-2 text-sm"
                : "mr-auto max-w-[70%] whitespace-pre-wrap rounded-2xl bg-white/10 px-4 py-2 text-sm"
            }
          >
            {message.text}
          </div>
        ))}
        {streaming ? (
          <div className="mr-auto max-w-[70%] space-y-2">
            {streaming.reasoning ? (
              <div className="whitespace-pre-wrap rounded-2xl border border-white/10 px-4 py-2 text-xs text-white/50">
                {streaming.reasoning}
              </div>
            ) : null}
            <div className="whitespace-pre-wrap rounded-2xl bg-white/10 px-4 py-2 text-sm">
              {streaming.answer || "…"}
            </div>
          </div>
        ) : null}
        <div ref={bottomRef} />
      </div>
      {error ? <div className="px-5 pb-2 text-xs text-red-400">{error}</div> : null}
      <footer className="border-t border-white/10 px-5 py-3">
        <textarea
          className="min-h-[64px] w-full resize-none rounded-xl bg-white/10 px-3 py-2 text-sm outline-none placeholder:text-white/40"
          placeholder={active.storedId ? "Message hermes…" : "Send a first message to create this chat…"}
          value={draft}
          disabled={busy && streaming == null}
          onChange={(event) => onDraftChange(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter" && !event.shiftKey) {
              event.preventDefault();
              onSend();
            }
          }}
        />
      </footer>
    </div>
  );
}
