const DEFAULT_RELAY_TIMEOUT_MS = 30_000;
const RELAY_HOST_ENDPOINTS_ENV = "FC_RELAY_HOST_ENDPOINTS_JSON";

export type RelayEndpointConfig = {
  baseUrl: string;
  adminToken: string;
};

type RelayEndpointEnv = Record<string, string | undefined>;

export function relayEndpointForSourceHost(
  sourceHostId: string | null | undefined,
  env: RelayEndpointEnv = process.env
): RelayEndpointConfig | null {
  const hostId = sourceHostId?.trim().toLowerCase();
  if (!hostId) {
    return null;
  }
  const raw = env[RELAY_HOST_ENDPOINTS_ENV]?.trim();
  if (!raw) {
    return null;
  }

  let parsed: unknown;
  try {
    parsed = JSON.parse(raw);
  } catch {
    throw new Error(`${RELAY_HOST_ENDPOINTS_ENV} must be valid JSON`);
  }
  if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) {
    throw new Error(`${RELAY_HOST_ENDPOINTS_ENV} must be an object keyed by source host id`);
  }

  const endpoint = (parsed as Record<string, unknown>)[hostId];
  if (endpoint === undefined) {
    return null;
  }
  if (!endpoint || typeof endpoint !== "object" || Array.isArray(endpoint)) {
    throw new Error(`${RELAY_HOST_ENDPOINTS_ENV}.${hostId} must be an endpoint object`);
  }
  const entry = endpoint as Record<string, unknown>;
  const baseUrl = stringField(entry.url) ?? stringField(entry.baseUrl);
  const adminToken = stringField(entry.adminToken) ?? stringField(entry.admin_token);
  if (!baseUrl || !adminToken) {
    throw new Error(`${RELAY_HOST_ENDPOINTS_ENV}.${hostId} requires url and adminToken`);
  }
  return { baseUrl, adminToken };
}

export type RelayHeartbeat = {
  ok: boolean;
  machineId: string;
  lastSeenAt: string;
};

export type RelayChatSnapshot<T = unknown> = {
  ok: boolean;
  machineId: string;
  snapshot: T;
  updatedAt: string;
};

export type RelayChatMessagePage<T = unknown> = {
  messages: T[];
  has_more: boolean;
  next_before: string | null;
};

export type RelayChatMessageInput = {
  body: string;
  client_message_id?: string | null;
  attachments?: unknown[];
};

export type RelayChatConversationInput = {
  projectAgentId: string;
  conversationId?: string | null;
  title?: string | null;
};

export type RelayChatConversationUpdateInput = {
  projectAgentId: string;
  title: string;
};

export type RelayBridgeIdentity = {
  bridgeAccountId: string;
  bridgeDeviceId: string;
};

export type RelayCommandScope = {
  machineId: string;
  roomId?: string | null;
  conversationId?: string | null;
  topicId?: string | null;
  projectId?: string | null;
  projectAgentId?: string | null;
  runtimeId?: string | null;
  targetDeviceId?: string | null;
  actorDeviceId?: string | null;
};

export type RelayCommandOptions = {
  bridge?: RelayBridgeIdentity | null;
  scope: RelayCommandScope;
  endpoint?: RelayEndpointConfig | null;
};

export type RelayStatusSnapshot<T = unknown> = {
  ok: boolean;
  machineId: string;
  stateKey: string;
  schema: string;
  revision: number;
  status?: T;
  error?: string;
  observedAt: string;
  expiresAt: string;
};

export async function callMachineRelay<T>(
  machineId: string,
  lane: string,
  kind: string,
  payload: Record<string, unknown>,
  options: RelayCommandOptions
): Promise<T> {
  if (!validRelayName(lane) || !validRelayName(kind)) {
    throw new Error("invalid relay lane or kind");
  }
  if (!options.scope) {
    throw new Error("relay command scope is required");
  }
  const machine = cleanRelayScopeId(machineId, "machineId");
  const scopeMachine = cleanRelayScopeId(options.scope.machineId, "machineId");
  if (scopeMachine !== machine) {
    throw new Error("relay command scope machineId mismatch");
  }
  const body: Record<string, unknown> = {
    lane,
    kind,
    ttlSecs: Math.ceil(relayTimeoutMs() / 1000),
    payload,
  };
  const bridge = options.bridge ? relayBridgeBody(options.bridge) : null;
  if (bridge) {
    body.bridge = bridge;
  }
  body.scope = relayCommandScopeBody(options.scope, bridge);

  const event = await relayFetch<{ id: string }>(
    `/api/finite/v1/machines/${encodeURIComponent(machine)}/events`,
    {
      method: "POST",
      body: JSON.stringify(body),
    },
    options.endpoint
  );

  const result = await relayFetch<{
    ok: boolean;
    output?: T;
    error?: string;
  }>(
    `/api/finite/v1/machines/${encodeURIComponent(machineId)}/results/${encodeURIComponent(
      event.id
    )}?waitMs=${relayTimeoutMs()}`,
    {
      method: "GET",
    },
    options.endpoint
  );

  if (!result.ok) {
    throw new Error(result.error || "relay request failed");
  }

  return result.output as T;
}

export async function callMachineChat<T>(
  machineId: string,
  kind: string,
  payload: Record<string, unknown>,
  options: RelayCommandOptions
): Promise<T> {
  if (!kind.startsWith("chat.")) {
    throw new Error("invalid chat relay kind");
  }

  return callMachineRelay<T>(machineId, "chat", kind, payload, options);
}

export function runtimeRelayScope(machineId: string): RelayCommandScope {
  const machine = cleanRelayScopeId(machineId, "machineId");
  return {
    machineId: machine,
    runtimeId: `runtime:${machine}`,
  };
}

export function conversationRelayScope(
  machineId: string,
  conversationId: string,
  input: Partial<Omit<RelayCommandScope, "machineId" | "conversationId">> = {}
): RelayCommandScope {
  return {
    ...runtimeRelayScope(machineId),
    ...cleanRelayScopeFields(input),
    conversationId: cleanRelayScopeId(conversationId, "conversationId"),
  };
}

export async function fetchMachineRelayHeartbeat(
  machineId: string,
  endpoint?: RelayEndpointConfig | null
): Promise<RelayHeartbeat | null> {
  try {
    return await relayFetch<RelayHeartbeat>(
      `/api/finite/v1/machines/${encodeURIComponent(machineId)}/heartbeat`,
      {
        method: "GET",
      },
      endpoint
    );
  } catch (error) {
    if (error instanceof Error && error.message.includes("404")) {
      return null;
    }
    throw error;
  }
}

export async function fetchMachineChatSnapshot<T>(
  machineId: string,
  endpoint?: RelayEndpointConfig | null
): Promise<RelayChatSnapshot<T> | null> {
  try {
    return await relayFetch<RelayChatSnapshot<T>>(
      `/api/finite/v1/machines/${encodeURIComponent(machineId)}/chat/snapshot`,
      {
        method: "GET",
      },
      endpoint
    );
  } catch (error) {
    if (error instanceof Error && error.message.includes("404")) {
      return null;
    }
    throw error;
  }
}

export async function fetchMachineChatConversations<T>(
  machineId: string,
  bridge: RelayBridgeIdentity,
  endpoint?: RelayEndpointConfig | null
): Promise<T[]> {
  const params = relayBridgeParams(bridge);
  return relayFetch<T[]>(
    `/api/finite/v1/machines/${encodeURIComponent(machineId)}/chat/conversations?${params.toString()}`,
    {
      method: "GET",
    },
    endpoint
  );
}

export async function createMachineChatConversation<T>(
  machineId: string,
  bridge: RelayBridgeIdentity,
  input: RelayChatConversationInput,
  endpoint?: RelayEndpointConfig | null
): Promise<T> {
  return relayFetch<T>(
    `/api/finite/v1/machines/${encodeURIComponent(machineId)}/chat/conversations`,
    {
      method: "POST",
      body: JSON.stringify({
        bridge: relayBridgeBody(bridge),
        projectAgentId: cleanRelayScopeId(input.projectAgentId, "projectAgentId"),
        ...(input.conversationId?.trim()
          ? { conversationId: cleanRelayScopeId(input.conversationId, "conversationId") }
          : {}),
        ...(input.title?.trim() ? { title: input.title.trim() } : {}),
      }),
    },
    endpoint
  );
}

export async function updateMachineChatConversation<T>(
  machineId: string,
  conversationId: string,
  bridge: RelayBridgeIdentity,
  input: RelayChatConversationUpdateInput,
  endpoint?: RelayEndpointConfig | null
): Promise<T> {
  return relayFetch<T>(
    `/api/finite/v1/machines/${encodeURIComponent(machineId)}/chat/conversations/${encodeURIComponent(
      cleanRelayScopeId(conversationId, "conversationId")
    )}`,
    {
      method: "PUT",
      body: JSON.stringify({
        bridge: relayBridgeBody(bridge),
        projectAgentId: cleanRelayScopeId(input.projectAgentId, "projectAgentId"),
        title: cleanChatConversationTitle(input.title),
      }),
    },
    endpoint
  );
}

export async function fetchMachineChatMessages<T>(
  machineId: string,
  conversationId: string,
  input: RelayBridgeIdentity & { projectAgentId?: string | null; limit?: number | null; before?: string | null },
  endpoint?: RelayEndpointConfig | null
): Promise<RelayChatMessagePage<T>> {
  const params = relayBridgeParams(input);
  if (input.projectAgentId?.trim()) {
    params.set("projectAgentId", input.projectAgentId.trim());
  }
  if (input.limit) {
    params.set("limit", String(input.limit));
  }
  if (input.before?.trim()) {
    params.set("before", input.before.trim());
  }
  const query = params.toString();
  return relayFetch<RelayChatMessagePage<T>>(
    `/api/finite/v1/machines/${encodeURIComponent(machineId)}/chat/conversations/${encodeURIComponent(conversationId)}/messages${query ? `?${query}` : ""}`,
    {
      method: "GET",
    },
    endpoint
  );
}

export async function sendMachineChatMessage<T>(
  machineId: string,
  conversationId: string,
  bridge: RelayBridgeIdentity,
  message: RelayChatMessageInput,
  endpoint?: RelayEndpointConfig | null
): Promise<T> {
  return relayFetch<T>(
    `/api/finite/v1/machines/${encodeURIComponent(machineId)}/chat/conversations/${encodeURIComponent(conversationId)}/messages`,
    {
      method: "POST",
      body: JSON.stringify({
        bridge: relayBridgeBody(bridge),
        message: {
          ...message,
          attachments: Array.isArray(message.attachments) ? message.attachments : [],
        },
      }),
    },
    endpoint
  );
}

export async function fetchMachineChatAttachment(
  machineId: string,
  attachmentId: string,
  bridge: RelayBridgeIdentity,
  endpoint?: RelayEndpointConfig | null
): Promise<Response> {
  const params = relayBridgeParams(bridge);
  const response = await fetch(
    new URL(
      `/api/finite/v1/machines/${encodeURIComponent(machineId)}/chat/attachments/${encodeURIComponent(attachmentId)}?${params.toString()}`,
      relayBaseUrl(endpoint)
    ),
    {
      cache: "no-store",
      headers: relayRequestHeaders(undefined, endpoint),
    }
  );
  if (!response.ok) {
    const text = await response.text();
    const parsed = parseRelayResponseText(text, false, response.status);
    const error = relayErrorMessage(parsed);
    throw new Error(error ? `${error} (${response.status})` : `relay attachment returned ${response.status}`);
  }
  return response;
}

function relayBridgeParams(bridge: RelayBridgeIdentity) {
  const body = relayBridgeBody(bridge);
  return new URLSearchParams(body);
}

function relayBridgeBody(bridge: RelayBridgeIdentity) {
  const bridgeAccountId = bridge.bridgeAccountId.trim();
  const bridgeDeviceId = bridge.bridgeDeviceId.trim();
  if (!bridgeAccountId || !bridgeDeviceId) {
    throw new Error("relay bridge identity is required");
  }
  return { bridgeAccountId, bridgeDeviceId };
}

function cleanChatConversationTitle(title: string) {
  const normalized = title.trim();
  if (!normalized) {
    throw new Error("conversation title is required");
  }
  if ([...normalized].length > 120) {
    throw new Error("conversation title is too long");
  }
  return normalized;
}

function relayCommandScopeBody(
  scope: RelayCommandScope,
  bridge: RelayBridgeIdentity | null
): RelayCommandScope {
  const body: RelayCommandScope = {
    ...cleanRelayScopeFields(scope),
    machineId: cleanRelayScopeId(scope.machineId, "machineId"),
  };
  if (!body.actorDeviceId && bridge) {
    body.actorDeviceId = bridge.bridgeDeviceId;
  }
  return body;
}

function cleanRelayScopeFields(
  scope: Partial<Omit<RelayCommandScope, "machineId">>
): Partial<Omit<RelayCommandScope, "machineId">> {
  return {
    ...(scope.roomId ? { roomId: cleanRelayScopeId(scope.roomId, "roomId") } : {}),
    ...(scope.conversationId
      ? { conversationId: cleanRelayScopeId(scope.conversationId, "conversationId") }
      : {}),
    ...(scope.topicId ? { topicId: cleanRelayScopeId(scope.topicId, "topicId") } : {}),
    ...(scope.projectId ? { projectId: cleanRelayScopeId(scope.projectId, "projectId") } : {}),
    ...(scope.projectAgentId
      ? { projectAgentId: cleanRelayScopeId(scope.projectAgentId, "projectAgentId") }
      : {}),
    ...(scope.runtimeId ? { runtimeId: cleanRelayScopeId(scope.runtimeId, "runtimeId") } : {}),
    ...(scope.targetDeviceId
      ? { targetDeviceId: cleanRelayScopeId(scope.targetDeviceId, "targetDeviceId") }
      : {}),
    ...(scope.actorDeviceId
      ? { actorDeviceId: cleanRelayScopeId(scope.actorDeviceId, "actorDeviceId") }
      : {}),
  };
}

function cleanRelayScopeId(value: string, label: string) {
  const trimmed = value.trim();
  if (!trimmed) {
    throw new Error(`relay scope ${label} is required`);
  }
  if (!/^[A-Za-z0-9_.:-]+$/.test(trimmed)) {
    throw new Error(`invalid relay scope ${label}`);
  }
  return trimmed;
}

export async function fetchMachineStatusSnapshot<T>(
  machineId: string,
  stateKey: string,
  endpoint?: RelayEndpointConfig | null
): Promise<RelayStatusSnapshot<T> | null> {
  if (!validRelayName(stateKey)) {
    throw new Error("invalid relay status state key");
  }

  try {
    return await relayFetch<RelayStatusSnapshot<T>>(
      `/api/finite/v1/machines/${encodeURIComponent(machineId)}/status/snapshots/${encodeURIComponent(stateKey)}`,
      {
        method: "GET",
      },
      endpoint
    );
  } catch (error) {
    if (error instanceof Error && error.message.includes("404")) {
      return null;
    }
    throw error;
  }
}

export function relayStatusSnapshotValue<T>(
  snapshot: RelayStatusSnapshot<T> | null,
  expectedSchema: string,
  nowMs = Date.now()
): T | null {
  if (!snapshot) {
    return null;
  }
  if (snapshot.schema !== expectedSchema) {
    return null;
  }
  const expiresAtMs = Date.parse(snapshot.expiresAt);
  if (!Number.isFinite(expiresAtMs) || expiresAtMs <= nowMs) {
    return null;
  }
  if (!snapshot.ok) {
    throw new Error(snapshot.error || `${snapshot.stateKey} is unavailable`);
  }
  return snapshot.status ?? null;
}

export async function fetchMachineChatStream(
  machineId: string,
  bridge: RelayBridgeIdentity,
  lastEventId?: string | null,
  endpoint?: RelayEndpointConfig | null
): Promise<Response> {
  const params = relayBridgeParams(bridge);
  const headers = relayRequestHeaders(undefined, endpoint);
  if (lastEventId && lastEventId.trim().length > 0) {
    headers.set("last-event-id", lastEventId);
  }
  const response = await fetch(
    new URL(
      `/api/finite/v1/machines/${encodeURIComponent(machineId)}/chat/stream?${params.toString()}`,
      relayBaseUrl(endpoint)
    ),
    {
      cache: "no-store",
      headers,
    }
  );
  if (!response.ok) {
    const text = await response.text();
    throw new Error(
      text ? `${text.trim().slice(0, 500)} (${response.status})` : `relay stream returned ${response.status}`
    );
  }
  return response;
}

function validRelayName(value: string) {
  return /^[A-Za-z0-9_.-]+$/.test(value);
}

async function relayFetch<T>(
  path: string,
  init: RequestInit,
  endpoint?: RelayEndpointConfig | null
): Promise<T> {
  const response = await fetch(new URL(path, relayBaseUrl(endpoint)), {
    ...init,
    cache: "no-store",
    headers: relayRequestHeaders(init.headers, endpoint),
  });
  const text = await response.text();
  const parsed = parseRelayResponseText(text, response.ok, response.status);
  if (!response.ok) {
    const error = relayErrorMessage(parsed);
    throw new Error(error ? `${error} (${response.status})` : `relay service returned ${response.status}`, {
      cause: response.status,
    });
  }
  return parsed as T;
}

export function parseRelayResponseText(text: string, ok: boolean, status: number): unknown {
  try {
    return text ? JSON.parse(text) : {};
  } catch {
    if (!ok) {
      throw new Error(
        text ? `${text.trim().slice(0, 500)} (${status})` : `relay service returned ${status}`,
        { cause: status }
      );
    }
    throw new Error(`relay service returned non-JSON response (${status})`);
  }
}

function relayErrorMessage(value: unknown) {
  if (!value || typeof value !== "object" || !("error" in value)) {
    return null;
  }
  const error = (value as { error?: unknown }).error;
  return typeof error === "string" && error.trim() ? error : null;
}

function stringField(value: unknown) {
  return typeof value === "string" && value.trim() ? value.trim() : null;
}

function relayBaseUrl(endpoint?: RelayEndpointConfig | null) {
  const value = endpoint?.baseUrl?.trim() || process.env.FC_RELAY_URL?.trim();
  if (!value) {
    throw new Error("FC_RELAY_URL is required for chat relay");
  }
  return value.endsWith("/") ? value : `${value}/`;
}

function relayAdminToken(endpoint?: RelayEndpointConfig | null) {
  const value = endpoint?.adminToken?.trim() || process.env.FC_RELAY_ADMIN_TOKEN?.trim();
  if (!value) {
    throw new Error("FC_RELAY_ADMIN_TOKEN is required for chat relay");
  }
  return value;
}

function relayRequestHeaders(headers: HeadersInit | undefined, endpoint?: RelayEndpointConfig | null) {
  const next = new Headers(headers);
  next.set("authorization", `Bearer ${relayAdminToken(endpoint)}`);
  if (!next.has("content-type")) {
    next.set("content-type", "application/json");
  }
  return next;
}

function relayTimeoutMs() {
  const value = Number(process.env.FC_CHAT_RELAY_TIMEOUT_MS ?? DEFAULT_RELAY_TIMEOUT_MS);
  return Number.isFinite(value) ? Math.max(value, 1_000) : DEFAULT_RELAY_TIMEOUT_MS;
}
