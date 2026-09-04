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
 *                                       with (rides ?token=, same as hermes'
 *                                       own web UI). In a real deployment the
 *                                       token becomes per-viewer and this is
 *                                       the ONE injection point to change.
 *
 * Protocol facts this relies on (verified against hermes v2026.8.3):
 * - newline-delimited JSON-RPC; `gateway.ready` event on connect.
 * - Lists are PULL (`projects.tree`, `session.list`); turns are PUSH
 *   (`message.delta`/`message.complete`, `tool.*`, approvals). There is no
 *   session-list-change push: refresh after actions that change the list.
 * - `projects.tree` is hermes' own authoritative sidebar grouping: projects
 *   (≈ topics) → repos → lanes with preview sessions, plus the set of session
 *   ids claimed by projects so the rest render flat as Recents.
 * - A freshly created session is an in-memory draft with NO DB row — it is
 *   invisible to `session.list` until its first `prompt.submit`.
 */

import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { ChevronRightIcon, HashIcon, RotateCcwIcon } from "lucide-react";

const GATEWAY_WS_URL =
  process.env.NEXT_PUBLIC_HERMES_GATEWAY_WS_URL || "ws://127.0.0.1:9119/api/ws";
const GATEWAY_TOKEN = process.env.NEXT_PUBLIC_HERMES_GATEWAY_TOKEN || "";

type GatewayInbound = {
  jsonrpc: "2.0";
  id?: number;
  method?: string;
  params?: { type?: string } & Record<string, unknown>;
  result?: unknown;
  error?: { code?: number; message?: string };
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

export type GatewayConnection =
  | { state: "connecting" }
  | { state: "ready" }
  | { state: "error"; message: string };

/** One WebSocket, JSON-RPC correlation by id, auto-reconnect. */
function useHermesGateway() {
  const [connection, setConnection] = useState<GatewayConnection>({
    state: "connecting",
  });
  const [tree, setTree] = useState<GatewayTree>({ projects: [], scoped_session_ids: [] });
  const [sessions, setSessions] = useState<GatewaySession[]>([]);
  const socketRef = useRef<WebSocket | null>(null);
  const nextIdRef = useRef(1);
  const pendingRef = useRef(
    new Map<number, { resolve: (value: unknown) => void; reject: (reason: Error) => void }>()
  );

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
      setConnection({ state: "connecting" });
      const target = GATEWAY_TOKEN
        ? `${GATEWAY_WS_URL}?token=${encodeURIComponent(GATEWAY_TOKEN)}`
        : GATEWAY_WS_URL;
      const socket = new WebSocket(target);
      socketRef.current = socket;

      socket.addEventListener("open", () => {
        retryMs = 500;
        setConnection({ state: "ready" });
        void refreshRef.current().catch(() => undefined);
      });

      socket.addEventListener("message", (event: MessageEvent) => {
        let message: GatewayInbound;
        try {
          message = JSON.parse(String(event.data));
        } catch {
          return;
        }
        // Events (method === "event") are the turn stream; the parity probe's
        // transcript view will subscribe here. Responses correlate by id.
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
        setConnection({ state: "error", message: `gateway unreachable at ${GATEWAY_WS_URL}` });
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

  return { connection, tree, sessions, refresh };
}

/**
 * The hermes half of the side-by-side parity probe: projects (≈ topics) with
 * their sessions, then flat Recents for sessions no project claims — the same
 * split hermes' own desktop renders, from the same authoritative RPC.
 */
export function HermesGatewayChat() {
  const { connection, tree, sessions, refresh } = useHermesGateway();
  const [collapsed, setCollapsed] = useState<Set<string>>(() => new Set());

  const recents = useMemo(
    () => sessions.filter((session) => !tree.scoped_session_ids.includes(session.id)),
    [sessions, tree.scoped_session_ids]
  );

  // projects.tree includes hermes' zero-session "discovery tier" — every repo
  // it has ever seen a cwd for. The chat sidebar only cares about projects
  // that actually hold conversations.
  const projects = useMemo(
    () => tree.projects.filter((project) => project.sessionCount > 0),
    [tree.projects]
  );

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
          aria-label="Reload hermes sessions"
          title="Reload hermes sessions"
          onClick={() => void refresh().catch(() => undefined)}
        >
          <RotateCcwIcon className="size-3.5" />
        </button>
      </div>
      {connection.state === "connecting" ? (
        <p className="finite-agent-sidebar__status">Connecting to hermes gateway…</p>
      ) : null}
      {connection.state === "error" ? (
        <div className="finite-agent-sidebar__error">
          <span>{connection.message}</span>
        </div>
      ) : null}
      {connection.state === "ready" ? (
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
                <GatewaySessionRow key={session.id} session={session} title={rowTitle(session)} />
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
}: {
  session: GatewaySession;
  title: string;
}) {
  return (
    <div
      className="finite-chat__thread-row"
      title={`${session.preview}\n${session.message_count} messages · ${session.source || "unknown source"} · via tui_gateway`}
    >
      <div className="finite-chat__thread-open">
        <span className="finite-chat__thread-indicator" aria-hidden />
        <span className="finite-chat__thread-main">
          <span className="finite-chat__thread-title">{title}</span>
        </span>
      </div>
    </div>
  );
}
