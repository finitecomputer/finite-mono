/*
 * Test-only fixture for the direct chat bridge: a fake hosted-device signer,
 * a fake agent bridge behind a native Hermes base URL, and an injected Core
 * grant source with a controllable clock. Nothing here is imported by app code.
 */
import type { TestContext } from "node:test";

import { DirectChatBridge, type DirectChatAccess } from "@/lib/direct-chat-bridge";
import type { HostedHermesSession } from "@/lib/hosted-hermes-status";
import type { HostedChatState } from "@/lib/hosted-web-device";

export const HUMAN_ACCOUNT_ID = "11".repeat(32);
// NIP-19 test vector.
export const AGENT_ACCOUNT_ID = "3bf0c63fcb93463407af97a5e5ee64fa883d107ef9e558472c4eb9aaaefa459d";
export const AGENT_NPUB = "npub180cvv07tjdrrgpa0j7j7tmnyl2yr6yr7l8j4s3evf6u64th6gkwsyjh6w6";
export const NATIVE_BASE_URL = "https://agents-lat2.example/runtimes/runtime_1/";
export const BRIDGE_URL = `${NATIVE_BASE_URL}api/plugins/finitechat`;
export const DEVICE_URL = "https://device.internal";
export const WORKOS_ACCESS_TOKEN = "workos-access-token-fixture";
export const HOSTED_API_TOKEN = "hosted-device-token-fixture";

export const fixtureAccount = {
  email: "paul@finite.vip",
  workosUserId: "user_paul",
  emailVerified: true,
  accessToken: WORKOS_ACCESS_TOKEN,
  source: "workos" as const,
};

export const fixtureAccess: DirectChatAccess = {
  account: fixtureAccount,
  hosted: { baseUrl: DEVICE_URL, apiToken: HOSTED_API_TOKEN },
  runtimeId: "runtime_1",
  projectId: "project_1",
};

export type RecordedRequest = {
  url: string;
  method: string;
  headers: Headers;
  body: Uint8Array | null;
  signal: AbortSignal | null;
};

export function directInfo(overrides: Record<string, unknown> = {}) {
  return {
    transport: "direct",
    version: 1,
    agent_id: "agent_project_1",
    agent_account_id: AGENT_ACCOUNT_ID,
    rooms: ["room-1"],
    canonical_room_id: "room-1",
    migration: { state: "migrated", source: "agent-store" },
    ...overrides,
  };
}

export function fixtureState(overrides: Partial<HostedChatState> = {}): HostedChatState {
  return {
    rev: 7,
    identity: { account_id: HUMAN_ACCOUNT_ID, device_id: "direct" },
    rooms: [],
    topics: [{
      room_id: "room-1",
      topic_id: "home",
      title: "Home",
      last_message_preview: "",
      unread_count: 0,
      message_count: 0,
      created_seq: 0,
      updated_seq: 0,
      archived: false,
      chats: [],
    }],
    status: "ready",
    messages: [],
    profiles: [],
    devices: [],
    typing_members: [],
    flow: { notice_busy: false, scan_in_flight: false, scan_result: "" },
    ...overrides,
  };
}

const realFetch = globalThis.fetch;

/** Replaces global fetch for one test (installs may repeat) and records requests. */
export function installFetch(
  context: TestContext,
  handler: (request: RecordedRequest) => Response | Promise<Response>
) {
  const requests: RecordedRequest[] = [];
  globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
    const body = init?.body;
    const request: RecordedRequest = {
      url: String(input),
      method: init?.method ?? "GET",
      headers: new Headers(init?.headers),
      body: typeof body === "string"
        ? new TextEncoder().encode(body)
        : body instanceof Uint8Array
          ? body
          : null,
      signal: init?.signal ?? null,
    };
    requests.push(request);
    return handler(request);
  }) as typeof fetch;
  context.after(() => {
    globalThis.fetch = realFetch;
  });
  return requests;
}

/** The hosted device's direct-authorization response for one request. */
export function signedAuthorization(request: RecordedRequest, sequence: number) {
  const input = JSON.parse(new TextDecoder().decode(request.body!)) as {
    agent_id: string;
    method: string;
    path: string;
    body_sha256: string | null;
  };
  return Response.json({
    event: {
      id: `event-${sequence}`,
      pubkey: HUMAN_ACCOUNT_ID,
      created_at: 1_800_000_000,
      kind: 27235,
      tags: [
        ["u", `finitechat-direct://${input.agent_id}${input.path}`],
        ["method", input.method],
        ...(input.body_sha256 ? [["payload", input.body_sha256]] : []),
      ],
      content: "",
      sig: `sig-${sequence}`,
    },
    account_id: HUMAN_ACCOUNT_ID,
  });
}

/**
 * A fake world: the hosted device signs, the agent answers info and any
 * bridge route in `routes` (keyed by "METHOD /v1/app/path"), and anything
 * else is answered by `fallback` (404 by default).
 */
export function directWorld(
  context: TestContext,
  routes: Record<string, (request: RecordedRequest) => Response | Promise<Response>>,
  options: {
    info?: () => Response | Promise<Response>;
    fallback?: (request: RecordedRequest) => Response | Promise<Response>;
  } = {}
) {
  let signatures = 0;
  const requests = installFetch(context, (request) => {
    if (request.url === `${DEVICE_URL}/v1/chat/direct-authorization`) {
      return signedAuthorization(request, ++signatures);
    }
    if (request.url === `${BRIDGE_URL}/v1/app/info`) {
      return options.info ? options.info() : Response.json(directInfo());
    }
    if (request.url.startsWith(BRIDGE_URL)) {
      const url = new URL(request.url);
      const path = url.pathname.slice(new URL(BRIDGE_URL).pathname.length);
      const route = routes[`${request.method} ${path}`];
      if (route) return route(request);
    }
    return options.fallback
      ? options.fallback(request)
      : Response.json({ error: "not found" }, { status: 404 });
  });
  return {
    requests,
    signingRequests: () =>
      requests.filter((request) => request.url === `${DEVICE_URL}/v1/chat/direct-authorization`),
    agentRequests: () =>
      requests.filter((request) => request.url.startsWith(BRIDGE_URL) && !request.url.endsWith("/v1/app/info")),
    hostedAppRequests: () =>
      requests.filter((request) => request.url.startsWith(`${DEVICE_URL}/v1/app/`)),
  };
}

/** A bridge with an injected Core grant source and a controllable clock. */
export function fixtureBridge(
  options: {
    /** Throw to model a Core refusal; return a grant to override the default. */
    mint?: (
      runtimeId: string,
      userId: string
    ) => Promise<HostedHermesSession | undefined> | HostedHermesSession | undefined;
    grantLifetimeSeconds?: number;
  } = {}
) {
  let clock = 1_800_000_000_000;
  const mints: Array<{ userId: string | null; runtimeId: string }> = [];
  const bridge = new DirectChatBridge({
    now: () => clock,
    mintGrant: async (account, runtimeId) => {
      mints.push({ userId: account.workosUserId, runtimeId });
      const override = await options.mint?.(runtimeId, account.workosUserId ?? "");
      if (override) return override;
      return {
        baseUrl: NATIVE_BASE_URL,
        accessToken: `native-token-${mints.length}`,
        expiresAt: Math.floor(clock / 1000) + (options.grantLifetimeSeconds ?? 60),
      };
    },
  });
  return {
    bridge,
    mints,
    advance(ms: number) {
      clock += ms;
    },
  };
}
