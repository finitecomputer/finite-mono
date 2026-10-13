import { createHash } from "node:crypto";

import { CHAT_UNAVAILABLE_MESSAGE } from "@/lib/chat-product-copy";
import { CoreFetchError, createCoreHostedHermesSessionForAccount } from "@/lib/core-client";
import type { AccountAuthContext } from "@/lib/dashboard-auth";
import type { HostedHermesSession } from "@/lib/hosted-hermes-status";
import { HostedWebChatError } from "@/lib/hosted-web-chat-error";
import {
  hostedDeviceDiagnosticPath,
  hostedDeviceDirectChatAuthorization,
  hostedDeviceRequestError,
  type HostedDeviceConfig,
  type HostedRequesterContext,
} from "@/lib/hosted-web-device";

/*
 * Chat target resolution and the direct Finite Chat bridge client.
 *
 * An agent runs exactly one chat mode. Direct mode is capability detection:
 * the agent's own `GET /v1/app/info` (through the Core-authorized native
 * Hermes session) must report `transport: "direct"`. Everything else that is
 * evidence about the agent selects the legacy Hosted Web Device. Failures
 * that are not evidence (transport errors, 5xx, timeouts) never flip a mode:
 * a runtime last seen legacy stays legacy, and a runtime last seen direct or
 * never seen is unavailable. There is no hidden fallback from direct to the
 * hosted device's MLS path.
 *
 * Only runtimes the deployment names in `FC_DIRECT_CHAT_RUNTIME_IDS` (a
 * comma-separated list, or `*` for all) are probed; every other runtime stays
 * on the hosted device without contacting the agent. This keeps canaries and
 * staged rollouts explicit; the agent's own report still decides its mode.
 *
 * Native grants and decisions live in this process's memory only. The grant
 * is never returned to a browser and the WorkOS token never reaches an agent.
 */

export const DIRECT_CHAT_DECISION_TTL_MS = 30_000;
export const DIRECT_CHAT_GRANT_EXPIRY_MARGIN_MS = 10_000;
export const DIRECT_CHAT_AUTHORIZATION_HEADER = "x-finite-chat-authorization";
const DIRECT_CHAT_INFO_TIMEOUT_MS = 5_000;
const DIRECT_CHAT_MAX_CACHE_ENTRIES = 1024;
const DIRECT_CHAT_MAX_INFO_BYTES = 64 * 1024;
const DIRECT_CHAT_MAX_ROOMS = 256;
const DIRECT_CHAT_BRIDGE_PREFIX = "api/plugins/finitechat";
const DIRECT_CHAT_NOT_READY_MESSAGE =
  "This agent’s dashboard connection is not ready yet. Try again shortly.";
const ACCOUNT_ID_PATTERN = /^[0-9a-f]{64}$/u;

export type DirectChatAccess = {
  account: AccountAuthContext;
  hosted: HostedDeviceConfig;
  runtimeId: string;
  projectId: string;
};

export type DirectChatInfo = {
  version: 1;
  agentId: string;
  agentAccountId: string;
  rooms: string[];
  canonicalRoomId: string | null;
  migrationState: "migrated" | "fresh";
};

export type DirectChatTarget = { kind: "direct"; agentId: string; info: DirectChatInfo };
export type ChatTarget = { kind: "legacy" } | DirectChatTarget;

export type DirectBridgeRequest = {
  method: "GET" | "POST";
  /** Bridge path and query, starting `/v1/app/`. It is signed exactly as sent. */
  path: string;
  /** The exact body bytes; required for POST and forbidden for GET. */
  body?: string | Uint8Array<ArrayBuffer>;
  contentType?: string;
  accept?: string;
  /** Requester context for text sends, built for the signer's account. */
  requester?: (signerAccountId: string) => Promise<HostedRequesterContext | undefined>;
  signal?: AbortSignal;
  timeoutMs?: number;
};

export type DirectChatBridgeDeps = {
  /** Whether this runtime may use direct chat at all; defaults to the env. */
  eligible?: (runtimeId: string) => boolean;
  mintGrant: (
    account: AccountAuthContext,
    runtimeId: string,
    signal?: AbortSignal
  ) => Promise<HostedHermesSession>;
  now?: () => number;
};

type Observation =
  | { mode: "legacy"; observedAtMs: number }
  | { mode: "direct"; info: DirectChatInfo; observedAtMs: number };

type CachedGrant = { grant: HostedHermesSession; reuseUntilMs: number };

type InfoOutcome =
  | { mode: "direct"; info: DirectChatInfo }
  | { mode: "legacy"; agentReported: boolean }
  | { mode: "invalid"; reason: string };

const LEGACY: ChatTarget = { kind: "legacy" };

export class DirectChatBridge {
  private readonly observations = new Map<string, Observation>();
  private readonly probes = new Map<string, Promise<ChatTarget>>();
  private readonly grants = new Map<string, CachedGrant>();
  private readonly minting = new Map<string, Promise<HostedHermesSession>>();
  private readonly now: () => number;
  private readonly eligible: (runtimeId: string) => boolean;

  constructor(private readonly deps: DirectChatBridgeDeps) {
    this.now = deps.now ?? Date.now;
    this.eligible = deps.eligible ?? directChatEnabledFor;
  }

  /** Decide direct vs legacy for one machine; reused for 30 s per runtime. */
  async resolve(access: DirectChatAccess): Promise<ChatTarget> {
    if (!this.eligible(access.runtimeId)) return LEGACY;
    const seen = this.observations.get(access.runtimeId);
    if (seen && this.now() - seen.observedAtMs <= DIRECT_CHAT_DECISION_TTL_MS) {
      return targetFor(seen);
    }
    const key = accessKey(access);
    const pending = this.probes.get(key);
    if (pending) return pending;
    const probe = this.probe(access, seen).finally(() => this.probes.delete(key));
    this.probes.set(key, probe);
    return probe;
  }

  /** Drop a runtime's decision so the next request probes the agent again. */
  forget(runtimeId: string) {
    this.observations.delete(runtimeId);
  }

  /**
   * One signed bridge request. A native 401 refreshes the grant and retries
   * once with a fresh signature; any remaining 401 is answered as 403/503 so
   * the browser never mistakes agent authorization for an expired session.
   */
  async request(
    access: DirectChatAccess,
    target: DirectChatTarget,
    input: DirectBridgeRequest
  ): Promise<Response> {
    const path = directBridgePath(input.path);
    const body = typeof input.body === "string" ? new TextEncoder().encode(input.body) : input.body;
    if ((input.method === "POST") !== (body !== undefined)) {
      throw new HostedWebChatError("Invalid chat request.", 400);
    }
    const bodySha256 = body ? sha256Hex(body) : null;
    const signal = input.timeoutMs
      ? input.signal
        ? AbortSignal.any([input.signal, AbortSignal.timeout(input.timeoutMs)])
        : AbortSignal.timeout(input.timeoutMs)
      : input.signal;
    let requester: HostedRequesterContext | undefined;
    let requesterReady = false;
    for (let attempt = 0; ; attempt++) {
      const grant = await this.requestGrant(access);
      const authorization = await hostedDeviceDirectChatAuthorization(access.hosted, access.account, {
        agent_id: target.agentId,
        method: input.method,
        path,
        body_sha256: bodySha256,
      });
      if (input.requester && !requesterReady) {
        requester = await input.requester(authorization.accountId);
        requesterReady = true;
      }
      const headers = directBridgeHeaders(grant, authorization.event, input, requester);
      const response = await fetch(directBridgeUrl(grant.baseUrl, path), {
        method: input.method,
        body,
        headers,
        cache: "no-store",
        redirect: "error",
        signal,
      });
      if (response.status !== 401) return response;
      const text = await boundedText(response, DIRECT_CHAT_MAX_INFO_BYTES);
      if (isBridgeAuthorizationRejection(text)) {
        return new Response(text, {
          status: 403,
          headers: { "content-type": "application/json" },
        });
      }
      this.invalidateGrant(access, grant.accessToken);
      if (attempt > 0) {
        return Response.json(
          { error: CHAT_UNAVAILABLE_MESSAGE, error_kind: "agent_access_unavailable", retryable: true },
          { status: 503 }
        );
      }
    }
  }

  /** A JSON bridge call; failures keep the bridge's hosted-device error envelope. */
  async json<T>(
    access: DirectChatAccess,
    target: DirectChatTarget,
    input: DirectBridgeRequest
  ): Promise<T> {
    const diagnosticPath = hostedDeviceDiagnosticPath(input.path);
    let response: Response;
    try {
      response = await this.request(access, target, { accept: "application/json", ...input });
    } catch (error) {
      if (!(error instanceof HostedWebChatError)) {
        console.error("direct chat request failed", {
          path: diagnosticPath,
          errorClass:
            error instanceof DOMException && error.name === "TimeoutError" ? "timeout" : "transport",
        });
      }
      throw error;
    }
    if (!response.ok) {
      console.error("direct chat request failed", {
        path: diagnosticPath,
        status: response.status,
        errorClass: "downstream_http",
      });
      throw await hostedDeviceRequestError(response);
    }
    return response.json() as Promise<T>;
  }

  private async probe(access: DirectChatAccess, seen: Observation | undefined): Promise<ChatTarget> {
    let response: Response;
    try {
      response = await this.nativeGet(access, "/v1/app/info");
    } catch (error) {
      if (error instanceof HostedWebChatError) throw error;
      if (error instanceof CoreFetchError && [403, 404, 409].includes(error.status)) {
        // Core reports no hosted access for this owner/runtime: not direct.
        return this.notDirect(access, seen, error.status === 409);
      }
      return this.unknown(access, seen);
    }
    if (!response.ok) {
      await response.body?.cancel().catch(() => undefined);
      if (response.status === 404) {
        // Native Hermes accepted this grant and has no finitechat plugin route.
        this.observe(access.runtimeId, { mode: "legacy", observedAtMs: this.now() });
        return LEGACY;
      }
      if ([401, 403, 408, 429].includes(response.status) || response.status >= 500) {
        return this.unknown(access, seen);
      }
      return this.notDirect(access, seen, true);
    }
    let payload: unknown;
    try {
      payload = await boundedJson(response, DIRECT_CHAT_MAX_INFO_BYTES);
    } catch {
      return this.unknown(access, seen);
    }
    const outcome = classifyDirectChatInfo(payload, directChatAgentId(access.projectId));
    if (outcome.mode === "direct") {
      const observation = { mode: "direct" as const, info: outcome.info, observedAtMs: this.now() };
      this.observe(access.runtimeId, observation);
      return targetFor(observation);
    }
    if (outcome.mode === "invalid") {
      console.warn("direct chat info was rejected", { reason: outcome.reason });
      throw new HostedWebChatError(CHAT_UNAVAILABLE_MESSAGE, 503);
    }
    if (outcome.agentReported) {
      // The agent itself says it is not serving direct chat.
      this.observe(access.runtimeId, { mode: "legacy", observedAtMs: this.now() });
      return LEGACY;
    }
    return this.notDirect(access, seen, true);
  }

  /** Indirect evidence the agent is not direct; never overrides a direct agent. */
  private notDirect(access: DirectChatAccess, seen: Observation | undefined, cacheable: boolean) {
    if (seen?.mode === "direct") {
      throw new HostedWebChatError(DIRECT_CHAT_NOT_READY_MESSAGE, 503);
    }
    if (cacheable) {
      this.observe(access.runtimeId, { mode: "legacy", observedAtMs: this.now() });
    }
    return LEGACY;
  }

  /** No evidence either way: keep the last known legacy mode, else fail closed. */
  private unknown(access: DirectChatAccess, seen: Observation | undefined) {
    if (seen?.mode === "legacy") {
      this.observe(access.runtimeId, { mode: "legacy", observedAtMs: this.now() });
      return LEGACY;
    }
    throw new HostedWebChatError(CHAT_UNAVAILABLE_MESSAGE, 503);
  }

  private async nativeGet(access: DirectChatAccess, path: string) {
    for (let attempt = 0; ; attempt++) {
      const grant = await this.grant(access);
      const response = await fetch(directBridgeUrl(grant.baseUrl, path), {
        cache: "no-store",
        redirect: "error",
        headers: { accept: "application/json", authorization: `Bearer ${grant.accessToken}` },
        signal: AbortSignal.timeout(DIRECT_CHAT_INFO_TIMEOUT_MS),
      });
      if (response.status !== 401 || attempt > 0) return response;
      await response.body?.cancel().catch(() => undefined);
      this.invalidateGrant(access, grant.accessToken);
    }
  }

  /** The grant for a request on a runtime already known to be direct. */
  private async requestGrant(access: DirectChatAccess) {
    try {
      return await this.grant(access);
    } catch (error) {
      if (error instanceof HostedWebChatError) throw error;
      if (error instanceof CoreFetchError && [403, 404, 409].includes(error.status)) {
        throw new HostedWebChatError(DIRECT_CHAT_NOT_READY_MESSAGE, 503);
      }
      throw new HostedWebChatError(CHAT_UNAVAILABLE_MESSAGE, 503);
    }
  }

  /** Core-minted native grant, reused per (user, runtime) until expiresAt − 10 s. */
  private async grant(access: DirectChatAccess): Promise<HostedHermesSession> {
    const key = accessKey(access);
    const cached = this.grants.get(key);
    if (cached && this.now() < cached.reuseUntilMs) return cached.grant;
    const pending = this.minting.get(key);
    if (pending) return pending;
    const mint = this.deps
      .mintGrant(access.account, access.runtimeId)
      .then((grant) => {
        const reuseUntilMs = grant.expiresAt * 1000 - DIRECT_CHAT_GRANT_EXPIRY_MARGIN_MS;
        if (reuseUntilMs > this.now()) {
          boundedSet(this.grants, key, { grant, reuseUntilMs });
        } else {
          this.grants.delete(key);
        }
        return grant;
      })
      .catch((error: unknown) => {
        if (error instanceof CoreFetchError && error.status === 401) {
          throw new HostedWebChatError("Sign in again to use chat.", 401);
        }
        throw error;
      })
      .finally(() => this.minting.delete(key));
    this.minting.set(key, mint);
    return mint;
  }

  private invalidateGrant(access: DirectChatAccess, rejectedToken: string) {
    const key = accessKey(access);
    if (this.grants.get(key)?.grant.accessToken === rejectedToken) {
      this.grants.delete(key);
    }
  }

  private observe(runtimeId: string, observation: Observation) {
    boundedSet(this.observations, runtimeId, observation);
  }
}

/** Runtimes `FC_DIRECT_CHAT_RUNTIME_IDS` admits to direct chat probing. */
export function directChatEnabledFor(
  runtimeId: string,
  configured = process.env.FC_DIRECT_CHAT_RUNTIME_IDS ?? ""
) {
  const entries = configured
    .split(",")
    .map((entry) => entry.trim())
    .filter(Boolean);
  return entries.includes("*") || entries.includes(runtimeId);
}

/** The process-wide bridge used by chat and Connections routes. */
export const directChatBridge = new DirectChatBridge({
  mintGrant: createCoreHostedHermesSessionForAccount,
});

/** `agent_<project>`, the agent's FINITE_AGENT_ID as stamped by the Runner. */
export function directChatAgentId(projectId: string) {
  const sanitized = projectId
    .replace(/[^A-Za-z0-9_-]/gu, "-")
    .slice(0, 63)
    .replace(/^-+|-+$/gu, "");
  return `agent_${sanitized}`;
}

/**
 * The agent's one current conversation. A null canonical room is usable only
 * when it is unambiguous; several rooms fail closed rather than guessing.
 */
export function directChatCanonicalRoomId(info: DirectChatInfo): string {
  if (info.canonicalRoomId) return info.canonicalRoomId;
  if (info.rooms.length === 1) return info.rooms[0]!;
  if (info.rooms.length === 0) {
    throw new HostedWebChatError("Your chat is still getting ready. Try again shortly.", 503);
  }
  throw new HostedWebChatError(
    "Finite could not determine which conversation belongs to this agent.",
    409
  );
}

export function classifyDirectChatInfo(value: unknown, expectedAgentId: string): InfoOutcome {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    return { mode: "legacy", agentReported: false };
  }
  const info = value as Record<string, unknown>;
  if (typeof info.transport !== "string") return { mode: "legacy", agentReported: false };
  if (info.transport !== "direct") return { mode: "legacy", agentReported: true };
  const migrationState = info.migration && typeof info.migration === "object"
    ? (info.migration as Record<string, unknown>).state
    : undefined;
  // A pending fold keeps the sidecar on MLS; the agent is not direct yet.
  if (migrationState === "pending") return { mode: "legacy", agentReported: true };
  if (info.version !== 1) return { mode: "invalid", reason: "version" };
  if (info.agent_id !== expectedAgentId) return { mode: "invalid", reason: "agent_id" };
  if (typeof info.agent_account_id !== "string" || !ACCOUNT_ID_PATTERN.test(info.agent_account_id)) {
    return { mode: "invalid", reason: "agent_account_id" };
  }
  if (
    !Array.isArray(info.rooms) ||
    info.rooms.length > DIRECT_CHAT_MAX_ROOMS ||
    !info.rooms.every(validRoomId) ||
    new Set(info.rooms).size !== info.rooms.length
  ) {
    return { mode: "invalid", reason: "rooms" };
  }
  const rooms = info.rooms as string[];
  const canonical = info.canonical_room_id;
  const canonicalRoomId = canonical === null ? null : typeof canonical === "string" ? canonical : undefined;
  if (canonicalRoomId === undefined || (canonicalRoomId !== null && !rooms.includes(canonicalRoomId))) {
    return { mode: "invalid", reason: "canonical_room_id" };
  }
  if (migrationState !== "migrated" && migrationState !== "fresh") {
    return { mode: "invalid", reason: "migration" };
  }
  return {
    mode: "direct",
    info: {
      version: 1,
      agentId: expectedAgentId,
      agentAccountId: info.agent_account_id,
      rooms: [...rooms],
      canonicalRoomId,
      migrationState,
    },
  };
}

/** The header value: unpadded base64url of the signed event's JSON. */
export function encodeDirectChatAuthorization(event: Record<string, unknown>) {
  return Buffer.from(JSON.stringify(event), "utf8").toString("base64url");
}

export function sha256Hex(bytes: Uint8Array) {
  return createHash("sha256").update(bytes).digest("hex");
}

/** The exact multipart bytes and boundary for a form, built once and signed. */
export async function encodeMultipartBody(formData: FormData) {
  const encoded = new Request("https://direct-chat.invalid/", { method: "POST", body: formData });
  const contentType = encoded.headers.get("content-type");
  if (!contentType?.toLowerCase().startsWith("multipart/form-data;")) {
    throw new HostedWebChatError("Attachment upload is unavailable.", 400);
  }
  return { contentType, body: new Uint8Array(await encoded.arrayBuffer()) };
}

const BECH32_ALPHABET = "qpzry9x8gf2tvdw0s3jn54khce6mua7l";
const BECH32_GENERATORS = [0x3b6a57b2, 0x26508e6d, 0x1ea119fa, 0x3d4233dd, 0x2a1462b3];

/** NIP-19 npub for a 64-hex account id. */
export function npubFromAccountId(accountId: string) {
  if (!ACCOUNT_ID_PATTERN.test(accountId)) {
    throw new Error("Account id must be 64 lowercase hex characters.");
  }
  const words = convertBits(Buffer.from(accountId, "hex"), 8, 5);
  const hrp = "npub";
  const expanded = [
    ...[...hrp].map((character) => character.charCodeAt(0) >> 5),
    0,
    ...[...hrp].map((character) => character.charCodeAt(0) & 31),
  ];
  const polymod = bech32Polymod([...expanded, ...words, 0, 0, 0, 0, 0, 0]) ^ 1;
  const checksum = Array.from({ length: 6 }, (_, index) => (polymod >> (5 * (5 - index))) & 31);
  return `${hrp}1${[...words, ...checksum].map((word) => BECH32_ALPHABET[word]).join("")}`;
}

function bech32Polymod(values: number[]) {
  let checksum = 1;
  for (const value of values) {
    const top = checksum >>> 25;
    checksum = ((checksum & 0x1ffffff) << 5) ^ value;
    for (let bit = 0; bit < 5; bit++) {
      if ((top >>> bit) & 1) checksum ^= BECH32_GENERATORS[bit]!;
    }
  }
  return checksum;
}

function convertBits(data: Uint8Array, from: number, to: number) {
  let accumulator = 0;
  let bits = 0;
  const output: number[] = [];
  const mask = (1 << to) - 1;
  for (const value of data) {
    accumulator = ((accumulator << from) | value) & ((1 << (from + to - 1)) - 1);
    bits += from;
    while (bits >= to) {
      bits -= to;
      output.push((accumulator >> bits) & mask);
    }
  }
  if (bits > 0) output.push((accumulator << (to - bits)) & mask);
  return output;
}

function targetFor(observation: Observation): ChatTarget {
  return observation.mode === "direct"
    ? { kind: "direct", agentId: observation.info.agentId, info: observation.info }
    : LEGACY;
}

function accessKey(access: DirectChatAccess) {
  const user = access.account.workosUserId?.trim();
  if (!user) throw new HostedWebChatError("Sign in again to use chat.", 401);
  return `${user}\u0000${access.runtimeId}`;
}

/** Reject anything a URL parser could rewrite between signing and sending. */
function directBridgePath(path: string) {
  const [pathname] = path.split("?", 1);
  if (
    !path.startsWith("/v1/app/") ||
    Buffer.byteLength(path) > 2048 ||
    /[#\\\s]/u.test(path) ||
    pathname!.split("/").some((segment, index) => index > 0 && (segment === "" || segment === "." || segment === ".."))
  ) {
    throw new HostedWebChatError("Invalid chat request.", 400);
  }
  return path;
}

function directBridgeUrl(baseUrl: string, path: string) {
  const base = new URL(baseUrl);
  const url = new URL(`${DIRECT_CHAT_BRIDGE_PREFIX}${path}`, base);
  if (url.origin !== base.origin || `${url.pathname}${url.search}` !== `${base.pathname}${DIRECT_CHAT_BRIDGE_PREFIX}${path}`) {
    throw new HostedWebChatError("Invalid chat request.", 400);
  }
  return url;
}

function directBridgeHeaders(
  grant: HostedHermesSession,
  event: Record<string, unknown>,
  input: DirectBridgeRequest,
  requester: HostedRequesterContext | undefined
) {
  // Built from nothing: no browser, WorkOS, or hosted-device credential is forwarded.
  const headers = new Headers({
    accept: input.accept ?? "application/json",
    authorization: `Bearer ${grant.accessToken}`,
    [DIRECT_CHAT_AUTHORIZATION_HEADER]: encodeDirectChatAuthorization(event),
  });
  if (input.contentType) headers.set("content-type", input.contentType);
  if (requester) {
    headers.set("x-finite-requester-email", requester.email);
    headers.set("x-finite-sites-requester-assertion", requester.sitesAssertion);
  }
  return headers;
}

function isBridgeAuthorizationRejection(text: string) {
  try {
    const parsed = JSON.parse(text) as { error_kind?: unknown };
    return parsed?.error_kind === "chat_authorization_invalid";
  } catch {
    return false;
  }
}

function validRoomId(value: unknown) {
  return (
    typeof value === "string" &&
    value.trim().length > 0 &&
    Buffer.byteLength(value) <= 512 &&
    !/[\u0000-\u001f\u007f]/u.test(value)
  );
}

function boundedSet<V>(map: Map<string, V>, key: string, value: V) {
  map.delete(key);
  map.set(key, value);
  while (map.size > DIRECT_CHAT_MAX_CACHE_ENTRIES) {
    const oldest = map.keys().next().value;
    if (oldest === undefined) break;
    map.delete(oldest);
  }
}

async function boundedBytes(response: Response, maxBytes: number) {
  const reader = response.body?.getReader();
  if (!reader) return { bytes: Buffer.alloc(0), truncated: false };
  const chunks: Uint8Array[] = [];
  let size = 0;
  try {
    while (size <= maxBytes) {
      const { done, value } = await reader.read();
      if (done) break;
      size += value.length;
      chunks.push(value);
    }
  } finally {
    await reader.cancel().catch(() => undefined);
  }
  return { bytes: Buffer.concat(chunks).subarray(0, maxBytes), truncated: size > maxBytes };
}

async function boundedText(response: Response, maxBytes: number) {
  return new TextDecoder().decode((await boundedBytes(response, maxBytes)).bytes);
}

/** Parsed JSON, or null when the body is not JSON; oversized bodies throw. */
async function boundedJson(response: Response, maxBytes: number): Promise<unknown> {
  const { bytes, truncated } = await boundedBytes(response, maxBytes);
  if (truncated) throw new Error("direct chat info is too large");
  try {
    return JSON.parse(new TextDecoder().decode(bytes));
  } catch {
    return null;
  }
}
