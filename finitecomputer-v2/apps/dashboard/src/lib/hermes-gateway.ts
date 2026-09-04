/**
 * Spike: minimal client for the hermes-agent tui_gateway WebSocket transport
 * (tui_gateway/ws.py). One short-lived connection per request; the gateway
 * speaks newline-delimited JSON-RPC and emits `gateway.ready` on connect,
 * which we do not need to await before sending — dispatch is queued.
 *
 * Read-only surface for now (session.list); subscriptions to
 * message.delta/message.complete come with the updates half of the parity
 * probe. Connection info comes from env:
 *   HERMES_GATEWAY_WS_URL   default ws://127.0.0.1:9119/api/ws
 *   HERMES_GATEWAY_TOKEN    the HERMES_DASHBOARD_SESSION_TOKEN `serve` was
 *                           launched with (rides ?token=, like hermes' own UI)
 */

import type { HostedChatTopic } from "@/lib/hosted-web-device";

const DEFAULT_GATEWAY_WS_URL = "ws://127.0.0.1:9119/api/ws";
const REQUEST_TIMEOUT_MS = 5_000;

export class HermesGatewayError extends Error {}

export type HermesGatewaySession = {
  id: string;
  title?: string | null;
  preview?: string | null;
  started_at?: number;
  message_count?: number;
  source?: string | null;
};

type GatewayMessage = {
  jsonrpc: "2.0";
  id?: number;
  method?: string;
  result?: unknown;
  error?: { code?: number; message?: string };
};

export function hermesGatewayConfig(env: NodeJS.ProcessEnv = process.env) {
  return {
    url: env.HERMES_GATEWAY_WS_URL || DEFAULT_GATEWAY_WS_URL,
    token: env.HERMES_GATEWAY_TOKEN || "",
  };
}

async function gatewayWebSocket(): Promise<WebSocket> {
  const { url, token } = hermesGatewayConfig();
  const target = token ? `${url}?token=${encodeURIComponent(token)}` : url;
  const socket = new WebSocket(target);
  const failure = new Promise<never>((_, reject) => {
    const fail = () =>
      reject(
        new HermesGatewayError(
          `hermes gateway connection failed (${target.replace(/token=[^&]*/, "token=…")}); is \`hermes serve\` up and HERMES_GATEWAY_TOKEN set?`
        )
      );
    socket.addEventListener("error", fail, { once: true });
    socket.addEventListener("close", fail, { once: true });
  });
  const opened = new Promise<void>((resolve) => {
    socket.addEventListener("open", () => resolve(), { once: true });
  });
  await Promise.race([opened, failure, timeout("connect")]);
  return socket;
}

function timeout(stage: string): Promise<never> {
  return new Promise((_, reject) =>
    setTimeout(
      () => reject(new HermesGatewayError(`hermes gateway ${stage} timed out`)),
      REQUEST_TIMEOUT_MS
    )
  );
}

/** One JSON-RPC round trip over a fresh connection. */
export async function gatewayRpc(
  method: string,
  params: Record<string, unknown> = {}
): Promise<unknown> {
  const socket = await gatewayWebSocket();
  try {
    const id = 1;
    const response = new Promise<GatewayMessage>((resolve, reject) => {
      socket.addEventListener("message", (event: MessageEvent) => {
        let message: GatewayMessage;
        try {
          message = JSON.parse(String(event.data));
        } catch {
          return;
        }
        if (message.id === id) resolve(message);
      });
    });
    socket.send(JSON.stringify({ jsonrpc: "2.0", id, method, params }));
    const message = await Promise.race([response, timeout(method)]);
    if (message.error) {
      throw new HermesGatewayError(
        `hermes gateway ${method} failed: ${message.error.message ?? "unknown error"}`
      );
    }
    return message.result;
  } finally {
    socket.close();
  }
}

export async function hermesGatewaySessions(): Promise<HermesGatewaySession[]> {
  const result = await gatewayRpc("session.list");
  const sessions = (result as { sessions?: unknown } | null)?.sessions;
  if (!Array.isArray(sessions)) {
    throw new HermesGatewayError("hermes gateway session.list returned no sessions array");
  }
  return sessions as HermesGatewaySession[];
}

/**
 * Fold gateway sessions into the hosted-chat sidebar shape so the renderer
 * stays byte-identical between transports. One synthetic topic; each hermes
 * session is a chat. seq cursors have no gateway analogue yet, so they stay
 * zero and ordering falls to the caller.
 */
export async function hermesSessionsTopic(): Promise<HostedChatTopic> {
  const sessions = await hermesGatewaySessions();
  return {
    room_id: "hermes-gateway",
    topic_id: "sessions",
    title: "Hermes",
    description: "tui_gateway sessions via /api/ws",
    last_message_preview: sessions[0]?.preview ?? "",
    unread_count: 0,
    message_count: sessions.reduce((total, session) => total + (session.message_count ?? 0), 0),
    created_seq: 0,
    updated_seq: 0,
    archived: false,
    active_chat_id: null,
    chats: sessions.map((session) => ({
      chat_id: session.id,
      title: session.title || "Untitled session",
      last_message_preview: session.preview ?? "",
      unread_count: 0,
      message_count: session.message_count ?? 0,
      started_seq: 0,
      updated_seq: 0,
      active: false,
      archived: false,
    })),
  };
}
