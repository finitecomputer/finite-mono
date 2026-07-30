import { fetchRuntimeAgentNpub } from "@/lib/agent-contact";
import { CHAT_UNAVAILABLE_MESSAGE } from "@/lib/chat-product-copy";
import { getAccountAuthContext } from "@/lib/dashboard-auth";
import {
  dashboardMachineProjectFromSnapshot,
  loadDashboardMachineAccess,
} from "@/lib/dashboard-machine-access";
import {
  coreProjectLabel,
  coreProjectPrimaryUrl,
  loadCoreMe,
  type CoreVisibleProject,
} from "@/lib/core-client";
import {
  hostedDeviceAction,
  hostedDeviceAuthorizeAgentBinding,
  hostedDeviceAttachment,
  hostedDeviceAttachments,
  hostedDeviceConfig,
  hostedDeviceEnsureAgentBinding,
  hostedDeviceOpenAgentBinding,
  hostedDeviceReconcileDevice,
  hostedDeviceNewChat,
  hostedDeviceRuntimeCommand,
  hostedDeviceSearch,
  hostedDeviceState,
  hostedDeviceUpdates,
  HostedDeviceRequestError,
  type HostedChatAction,
  type HostedChatReference,
  type HostedChatReferenceSearchResult,
  type HostedChatSearchResult,
  type HostedChatState,
  type HostedRuntimeCommandResponse,
} from "@/lib/hosted-web-device";

const EMPTY_SCHEMA = "finite.agent.empty.request.v1";
const OWNER_CLAIM = "agent.owner.claim";
const AGENT_BINDING_AUTHORIZATION_REQUIRED =
  "first-time binding bootstrap was not authorized by Project creation";
const AGENT_BINDING_RECOVERY_REQUIRED =
  `canonical Agent conversation requires recovery: ${AGENT_BINDING_AUTHORIZATION_REQUIRED}`;
const MAX_DEVICE_ID_BYTES = 128;
const MAX_CHAT_REFERENCE_COUNT = 12;

export const MAX_HOSTED_DEVICE_RECONCILE_REQUEST_BYTES = 4 * 1024;

export class HostedWebChatError extends Error {
  constructor(
    message: string,
    readonly status: number,
    readonly code?: "binding_authorization_required"
  ) {
    super(message);
  }
}

export function hostedWebChatErrorMessage(error: unknown) {
  return error instanceof HostedWebChatError ? error.message : CHAT_UNAVAILABLE_MESSAGE;
}

export function isAgentBindingAuthorizationRequired(error: unknown) {
  return (
    error instanceof HostedDeviceRequestError &&
    ((error.status === 409 && error.message === AGENT_BINDING_AUTHORIZATION_REQUIRED) ||
      (error.status === 503 && error.message === AGENT_BINDING_RECOVERY_REQUIRED))
  );
}

export async function bootstrapHostedWebChat(machineId: string) {
  const context = await hostedWebChatContext(machineId);
  return bootstrapHostedWebChatWithContext(context);
}

async function bootstrapHostedWebChatWithContext(
  context: Awaited<ReturnType<typeof hostedWebChatContext>>
) {
  try {
    return await hostedDeviceOpenAgentBinding(context.config, context.account, context.projectId);
  } catch (error) {
    if (!(error instanceof HostedDeviceRequestError) || error.status !== 404) {
      throw error;
    }
  }

  const state = await hostedDeviceState(context.config, context.account);
  await ensureRuntimeStarted(context, state);

  const agentNpub = await fetchRuntimeAgentNpub(context.primaryUrl);
  if (!agentNpub) {
    throw new HostedWebChatError("Your agent is still getting ready. Try again shortly.", 503);
  }
  try {
    return await hostedDeviceEnsureAgentBinding(context.config, context.account, {
      project_id: context.projectId,
      agent_npub: agentNpub,
      display_name: `Chat with ${context.agentName}`,
    });
  } catch (error) {
    if (isAgentBindingAuthorizationRequired(error)) {
      throw new HostedWebChatError(
        "Finish chat setup to continue.",
        409,
        "binding_authorization_required"
      );
    }
    throw error;
  }
}

export async function recoverHostedWebChatBinding(machineId: string) {
  const account = await getAccountAuthContext();
  if (!account.workosUserId || !account.emailVerified) {
    throw new HostedWebChatError("Sign in again to finish chat setup.", 401);
  }
  const core = await loadCoreMe();
  if (
    core.account.workosUserId !== account.workosUserId ||
    !core.account.emailVerified
  ) {
    throw new HostedWebChatError("Sign in again to finish chat setup.", 401);
  }
  const project = dashboardMachineProjectFromSnapshot(core.me, machineId);
  if (!project?.runtime) {
    throw new HostedWebChatError(
      "Finite could not verify this agent from one current Core snapshot.",
      409
    );
  }
  const context = hostedWebChatContextForProject(account, project);
  const requests = (core.me?.agent_creation_requests ?? []).filter(
    (candidate) =>
      candidate.project_id === context.projectId &&
      ["requested", "launching", "running"].includes(candidate.status)
  );
  if (!project || requests.length !== 1) {
    throw new HostedWebChatError(
      "Finite could not verify the original agent creation request.",
      409
    );
  }
  const creation = requests[0]!;
  await hostedDeviceAuthorizeAgentBinding(context.config, context.account, {
    project_id: context.projectId,
    creation_request_id: creation.id,
  });
  // Keep the Project, Runtime contact, and Agent Principal lookup on the same
  // fresh Core snapshot that authorized this exact recovery. Falling back to
  // the ordinary SWR context here could bind a stale Runtime endpoint.
  return bootstrapHostedWebChatWithContext(context);
}

export async function claimHostedWebChatOwner(machineId: string) {
  const context = await hostedWebChatContext(machineId);
  const state = await bootstrapHostedWebChatWithContext(context);
  const binding = state.hosted_agent_binding;
  if (!binding) {
    throw new HostedWebChatError("Your chat is still getting ready. Try again shortly.", 503);
  }
  await claimAgentOwner(context, state, binding.agent_account_id, binding.canonical_room_id);
  return { claimed: true as const };
}

export async function reconcileHostedWebChatDevice(
  machineId: string,
  targetDeviceId: string
) {
  const context = await hostedWebChatContext(machineId);
  return hostedDeviceReconcileDevice(context.config, context.account, {
    project_id: context.projectId,
    target_device_id: targetDeviceId,
  });
}

export function parseHostedDeviceReconcileRequest(payload: unknown) {
  const record = objectRecord(payload, "Device reconciliation request");
  const keys = Object.keys(record);
  if (keys.length !== 1 || keys[0] !== "target_device_id") {
    throw new HostedWebChatError("Invalid Device reconciliation request.", 400);
  }
  const targetDeviceId = boundedDeviceId(record.target_device_id);
  if (targetDeviceId === "hosted-web") {
    throw new HostedWebChatError("Invalid target_device_id.", 400);
  }
  return { target_device_id: targetDeviceId };
}

export async function parseHostedDeviceReconcileJsonRequest(request: Request) {
  const mediaType = (request.headers.get("content-type") ?? "")
    .split(";", 1)[0]
    .trim()
    .toLowerCase();
  if (mediaType !== "application/json") {
    throw new HostedWebChatError("Device reconciliation requests must use JSON.", 415);
  }

  const contentLength = request.headers.get("content-length");
  if (contentLength !== null) {
    const declaredLength = Number(contentLength);
    if (!Number.isSafeInteger(declaredLength) || declaredLength < 0) {
      throw new HostedWebChatError("Invalid Device reconciliation request.", 400);
    }
    if (declaredLength > MAX_HOSTED_DEVICE_RECONCILE_REQUEST_BYTES) {
      throw new HostedWebChatError("Device reconciliation request is too large.", 413);
    }
  }
  if (!request.body) {
    throw new HostedWebChatError("Invalid Device reconciliation request.", 400);
  }

  const reader = request.body.getReader();
  const chunks: Uint8Array[] = [];
  let totalBytes = 0;
  while (true) {
    const { done, value } = await reader.read();
    if (done) break;
    totalBytes += value.byteLength;
    if (totalBytes > MAX_HOSTED_DEVICE_RECONCILE_REQUEST_BYTES) {
      await reader.cancel().catch(() => undefined);
      throw new HostedWebChatError("Device reconciliation request is too large.", 413);
    }
    chunks.push(value);
  }

  const encoded = new Uint8Array(totalBytes);
  let offset = 0;
  for (const chunk of chunks) {
    encoded.set(chunk, offset);
    offset += chunk.byteLength;
  }
  let payload: unknown;
  try {
    payload = JSON.parse(new TextDecoder("utf-8", { fatal: true }).decode(encoded));
  } catch {
    throw new HostedWebChatError("Invalid Device reconciliation request.", 400);
  }
  return parseHostedDeviceReconcileRequest(payload);
}

export async function dispatchHostedWebChatAction(machineId: string, payload: unknown) {
  const context = await hostedWebChatContext(machineId);
  const action = parseHostedChatAction(payload);
  if ("StartTopicChatIntent" in action) {
    const bound = await hostedDeviceOpenAgentBinding(
      context.config,
      context.account,
      context.projectId
    );
    const target = action.StartTopicChatIntent;
    if (!isCanonicalNewChatTarget(bound, target)) {
      throw new HostedWebChatError("New chats must stay in the Agent conversation.", 409);
    }
    return hostedDeviceNewChat(context.config, context.account, {
      project_id: context.projectId,
      ...target,
    });
  }
  return hostedDeviceAction(context.config, context.account, action);
}

export async function searchHostedWebChats(
  machineId: string,
  payload: unknown
): Promise<HostedChatSearchResult[]> {
  const context = await hostedWebChatContext(machineId);
  const input = parseHostedChatSearchRequest(payload);
  const bound = await hostedDeviceOpenAgentBinding(
    context.config,
    context.account,
    context.projectId
  );
  if (bound.hosted_agent_binding?.canonical_room_id !== input.room_id) {
    throw new HostedWebChatError("Chat search must stay in the Agent conversation.", 409);
  }
  return hostedDeviceSearch(context.config, context.account, {
    ...input,
    limit: 30,
  });
}

export async function searchHostedChatReferences(
  machineId: string,
  payload: unknown
): Promise<HostedChatReferenceSearchResult[]> {
  const context = await hostedWebChatContext(machineId);
  const input = parseHostedChatReferenceSearchRequest(payload);
  const bound = await hostedDeviceOpenAgentBinding(
    context.config,
    context.account,
    context.projectId
  );
  const binding = bound.hosted_agent_binding;
  if (
    !binding
    || binding.canonical_room_id !== input.room_id
    || !bound.topics.some(
      (topic) =>
        topic.room_id === binding.canonical_room_id
        && topic.topic_id === input.topic_id
    )
  ) {
    throw new HostedWebChatError("Reference search must stay in the Agent conversation.", 409);
  }
  const response = await hostedDeviceRuntimeCommand(context.config, context.account, {
    room_id: binding.canonical_room_id,
    conversation_id: input.topic_id,
    target_account_id: binding.agent_account_id,
    command: "agent.context.search",
    schema: "finite.agent.context.search.v1",
    body: { query: input.query, limit: 30 },
    wait_millis: 15_000,
  });
  assertCommandSucceeded(response);
  return parseHostedChatReferenceSearchResults(response.body);
}

export function parseHostedChatReferenceSearchRequest(payload: unknown) {
  const record = objectRecord(payload, "Chat reference search request");
  const query = typeof record.query === "string" ? record.query.trim() : "";
  const roomId = typeof record.room_id === "string" ? record.room_id : "";
  const topicId = typeof record.topic_id === "string" ? record.topic_id : "";
  if (
    Object.keys(record).some((key) => !["query", "room_id", "topic_id"].includes(key))
    || query.length > 128
    || !roomId
    || !topicId
  ) {
    throw new HostedWebChatError("Invalid chat reference search request.", 400);
  }
  return { room_id: roomId, topic_id: topicId, query };
}

function parseHostedChatReferenceSearchResults(
  payload: unknown
): HostedChatReferenceSearchResult[] {
  if (!payload || typeof payload !== "object") {
    throw new HostedWebChatError("The Agent returned an invalid reference catalog.", 502);
  }
  const results = (payload as { results?: unknown }).results;
  if (!Array.isArray(results) || results.length > 40) {
    throw new HostedWebChatError("The Agent returned an invalid reference catalog.", 502);
  }
  return results.map((value) => {
    if (!value || typeof value !== "object") {
      throw new HostedWebChatError("The Agent returned an invalid reference.", 502);
    }
    const entry = value as Record<string, unknown>;
    if (
      (entry.kind !== "file" && entry.kind !== "skill" && entry.kind !== "site")
      || typeof entry.id !== "string"
      || typeof entry.label !== "string"
      || typeof entry.detail !== "string"
      || typeof entry.updated_at_ms !== "number"
    ) {
      throw new HostedWebChatError("The Agent returned an invalid reference.", 502);
    }
    return {
      kind: entry.kind,
      id: entry.id,
      label: entry.label,
      detail: entry.detail,
      path: typeof entry.path === "string" ? entry.path : null,
      description: typeof entry.description === "string" ? entry.description : null,
      url: typeof entry.url === "string" ? entry.url : null,
      fingerprint: typeof entry.fingerprint === "string" ? entry.fingerprint : null,
      updated_at_ms: entry.updated_at_ms,
    };
  });
}

export function parseHostedChatSearchRequest(payload: unknown) {
  const record = objectRecord(payload, "Chat search request");
  const query = typeof record.query === "string" ? record.query.trim() : "";
  const roomId = typeof record.room_id === "string" ? record.room_id : "";
  if (
    Object.keys(record).some((key) => !["query", "room_id"].includes(key))
    || query.length < 2
    || query.length > 256
    || !roomId
  ) {
    throw new HostedWebChatError("Invalid chat search request.", 400);
  }
  return { room_id: roomId, query };
}

export function isCanonicalNewChatTarget(
  state: HostedChatState,
  target: Extract<HostedChatAction, { StartTopicChatIntent: unknown }>["StartTopicChatIntent"]
) {
  const binding = state.hosted_agent_binding;
  return Boolean(
    binding
    && target.room_id === binding.canonical_room_id
    && state.topics.some(
      (topic) =>
        topic.room_id === binding.canonical_room_id
        && topic.topic_id === target.topic_id
    )
  );
}

export async function streamHostedWebChat(machineId: string, signal: AbortSignal) {
  const context = await hostedWebChatContext(machineId);
  return hostedDeviceUpdates(context.config, context.account, signal);
}

export async function uploadHostedWebChatAttachments(machineId: string, formData: FormData) {
  const context = await hostedWebChatContext(machineId);
  return hostedDeviceAttachments(context.config, context.account, formData);
}

export async function streamHostedWebChatAttachment(
  machineId: string,
  roomId: string,
  messageId: string,
  attachmentId: string,
  signal: AbortSignal
) {
  const context = await hostedWebChatContext(machineId);
  return hostedDeviceAttachment(
    context.config,
    context.account,
    roomId,
    messageId,
    attachmentId,
    signal
  );
}

async function hostedWebChatContext(
  machineId: string,
  coreCacheMode: "fresh" | "swr" = "swr"
) {
  const account = await getAccountAuthContext();
  if (!account.workosUserId || !account.emailVerified) {
    throw new HostedWebChatError("Sign in again to use chat.", 401);
  }
  const access = await loadDashboardMachineAccess(machineId, { coreCacheMode });
  if (!access) {
    throw new HostedWebChatError("Agent not found.", 404);
  }
  return hostedWebChatContextForProject(account, access.coreProject);
}

function hostedWebChatContextForProject(
  account: Awaited<ReturnType<typeof getAccountAuthContext>>,
  project: CoreVisibleProject
) {
  const config = hostedDeviceConfig();
  if (!config) {
    throw new HostedWebChatError(CHAT_UNAVAILABLE_MESSAGE, 503);
  }
  const runtime = project.runtime;
  if (!runtime) {
    throw new HostedWebChatError("Agent not found.", 404);
  }
  return {
    account,
    config,
    primaryUrl: coreProjectPrimaryUrl(project),
    agentName: coreProjectLabel(project),
    projectId: project.project.id,
    runtimeId: runtime.id,
  };
}

async function claimAgentOwner(
  context: Awaited<ReturnType<typeof hostedWebChatContext>>,
  state: HostedChatState,
  agentAccountId: string,
  canonicalRoomId: string
) {
  if (!state.rooms.some((room) => room.room_id === canonicalRoomId)) {
    throw new HostedWebChatError("Your chat is still getting ready. Try again shortly.", 503);
  }
  const response = await hostedDeviceRuntimeCommand(context.config, context.account, {
    room_id: canonicalRoomId,
    target_account_id: agentAccountId,
    command: OWNER_CLAIM,
    resource_key: "agent.connections",
    schema: EMPTY_SCHEMA,
    body: {},
    reuse_succeeded_owner_claim: true,
    wait_millis: 45_000,
  });
  assertCommandSucceeded(response);
}

function assertCommandSucceeded(response: HostedRuntimeCommandResponse) {
  if (response.status === "succeeded") {
    return;
  }
  throw new HostedWebChatError(
    response.error?.message || "Your chat is not ready yet. Try again shortly.",
    response.error?.code === "unauthorized" ? 403 : 502
  );
}

async function ensureRuntimeStarted(
  context: Awaited<ReturnType<typeof hostedWebChatContext>>,
  state: HostedChatState
) {
  if (state.status.toLowerCase().includes("running")) {
    return state;
  }
  return hostedDeviceAction(context.config, context.account, { StartRuntime: null });
}

export function parseHostedChatAction(payload: unknown): HostedChatAction {
  const record = objectRecord(payload, "chat action");
  const keys = Object.keys(record);
  if (keys.length !== 1) {
    throw new HostedWebChatError("Chat action must contain exactly one operation.", 400);
  }
  const operation = keys[0];
  const input = record[operation];

  switch (operation) {
    case "StartRuntime":
      if (input !== null) {
        throw new HostedWebChatError("That chat action is not available.", 400);
      }
      return { StartRuntime: null };
    case "OpenRoom": {
      const value = objectRecord(input, operation);
      return { OpenRoom: { room_id: boundedString(value.room_id, "room_id") } };
    }
    case "OpenTopic": {
      const value = objectRecord(input, operation);
      return {
        OpenTopic: {
          room_id: boundedString(value.room_id, "room_id"),
          topic_id: boundedString(value.topic_id, "topic_id"),
        },
      };
    }
    case "OpenChat": {
      const value = objectRecord(input, operation);
      return {
        OpenChat: {
          room_id: boundedString(value.room_id, "room_id"),
          topic_id: boundedString(value.topic_id, "topic_id"),
          chat_id: boundedString(value.chat_id, "chat_id"),
        },
      };
    }
    case "CreateTopic": {
      const value = objectRecord(input, operation);
      return {
        CreateTopic: {
          room_id: boundedString(value.room_id, "room_id"),
          title: boundedString(value.title, "title", 256),
        },
      };
    }
    case "StartTopicChatIntent": {
      const value = objectRecord(input, operation);
      return {
        StartTopicChatIntent: {
          room_id: boundedString(value.room_id, "room_id"),
          topic_id: boundedString(value.topic_id, "topic_id"),
          reason: optionalBoundedString(value.reason, "reason", 256),
          intent_key: boundedString(value.intent_key, "intent_key", 256),
        },
      };
    }
    case "RenameChat": {
      const value = objectRecord(input, operation);
      return {
        RenameChat: {
          room_id: boundedString(value.room_id, "room_id"),
          topic_id: boundedString(value.topic_id, "topic_id"),
          chat_id: boundedString(value.chat_id, "chat_id"),
          title: boundedString(value.title, "title", 256),
        },
      };
    }
    case "SetChatArchived": {
      const value = objectRecord(input, operation);
      if (typeof value.archived !== "boolean") {
        throw new HostedWebChatError("archived must be a boolean.", 400);
      }
      return {
        SetChatArchived: {
          room_id: boundedString(value.room_id, "room_id"),
          topic_id: boundedString(value.topic_id, "topic_id"),
          chat_id: boundedString(value.chat_id, "chat_id"),
          archived: value.archived,
        },
      };
    }
    case "SendMessage": {
      const value = objectRecord(input, operation);
      return {
        SendMessage: {
          room_id: boundedString(value.room_id, "room_id"),
          text: boundedString(value.text, "text", 64 * 1024),
        },
      };
    }
    case "SendTopicMessage": {
      const value = objectRecord(input, operation);
      return {
        SendTopicMessage: {
          room_id: boundedString(value.room_id, "room_id"),
          topic_id: boundedString(value.topic_id, "topic_id"),
          text: boundedString(value.text, "text", 64 * 1024),
        },
      };
    }
    case "SendChatMessage": {
      const value = objectRecord(input, operation);
      return {
        SendChatMessage: {
          room_id: boundedString(value.room_id, "room_id"),
          topic_id: boundedString(value.topic_id, "topic_id"),
          chat_id: boundedString(value.chat_id, "chat_id"),
          text: boundedString(value.text, "text", 64 * 1024),
        },
      };
    }
    case "SendChatMessageWithReferences": {
      const value = objectRecord(input, operation);
      return {
        SendChatMessageWithReferences: {
          room_id: boundedString(value.room_id, "room_id"),
          topic_id: boundedString(value.topic_id, "topic_id"),
          chat_id: boundedString(value.chat_id, "chat_id"),
          text: boundedString(value.text, "text", 64 * 1024),
          references: parseChatReferences(value.references),
        },
      };
    }
    case "LoadOlderMessages": {
      const value = objectRecord(input, operation);
      return {
        LoadOlderMessages: {
          room_id: boundedString(value.room_id, "room_id"),
          before_message_id: boundedString(value.before_message_id, "before_message_id"),
          limit: boundedInteger(value.limit, "limit", 1, 100),
        },
      };
    }
    case "MarkRoomRead": {
      const value = objectRecord(input, operation);
      return { MarkRoomRead: { room_id: boundedString(value.room_id, "room_id") } };
    }
    case "SetTyping": {
      const value = objectRecord(input, operation);
      if (typeof value.is_typing !== "boolean") {
        throw new HostedWebChatError("Invalid is_typing.", 400);
      }
      return {
        SetTyping: {
          room_id: boundedString(value.room_id, "room_id"),
          is_typing: value.is_typing,
        },
      };
    }
    case "RefreshDevices":
      if (input !== null) {
        throw new HostedWebChatError("That chat action is not available.", 400);
      }
      return { RefreshDevices: null };
    case "RevokeDevice": {
      const value = objectRecord(input, operation);
      return {
        RevokeDevice: {
          account_id: boundedString(value.account_id, "account_id"),
          device_id: boundedString(value.device_id, "device_id"),
        },
      };
    }
    default:
      throw new HostedWebChatError(`Unsupported chat action: ${operation}`, 400);
  }
}

function parseChatReferences(value: unknown): HostedChatReference[] {
  if (
    !Array.isArray(value)
    || value.length === 0
    || value.length > MAX_CHAT_REFERENCE_COUNT
  ) {
    throw new HostedWebChatError("Invalid chat references.", 400);
  }
  return value.map((candidate) => {
    const reference = objectRecord(candidate, "chat reference");
    if (
      reference.kind !== "file"
      && reference.kind !== "skill"
      && reference.kind !== "site"
    ) {
      throw new HostedWebChatError("Invalid chat reference kind.", 400);
    }
    const kind = reference.kind;
    const path = optionalBoundedString(reference.path, "reference.path", 1024);
    const url = optionalBoundedString(reference.url, "reference.url", 2048);
    const fingerprint = optionalBoundedString(
      reference.fingerprint,
      "reference.fingerprint",
      256
    );
    if (reference.kind === "file" && !path) {
      throw new HostedWebChatError("File references require a path.", 400);
    }
    if (reference.kind === "site" && !url) {
      throw new HostedWebChatError("Site references require a URL.", 400);
    }
    return {
      kind,
      id: boundedString(reference.id, "reference.id", 512),
      label: boundedString(reference.label, "reference.label", 256),
      detail: boundedString(reference.detail, "reference.detail", 1024),
      token: boundedString(reference.token, "reference.token", 512),
      ...(path ? { path } : {}),
      ...(url ? { url } : {}),
      ...(fingerprint ? { fingerprint } : {}),
    };
  });
}

function objectRecord(value: unknown, label: string): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new HostedWebChatError(`Invalid ${label}.`, 400);
  }
  return value as Record<string, unknown>;
}

function boundedString(value: unknown, label: string, maxBytes = 512) {
  if (typeof value !== "string" || !value.trim() || Buffer.byteLength(value) > maxBytes) {
    throw new HostedWebChatError(`Invalid ${label}.`, 400);
  }
  return value;
}

function boundedDeviceId(value: unknown) {
  if (
    typeof value !== "string" ||
    !value ||
    value.trim() !== value ||
    new TextEncoder().encode(value).byteLength > MAX_DEVICE_ID_BYTES ||
    /[\p{Cc}\p{Cf}]/u.test(value)
  ) {
    throw new HostedWebChatError("Invalid target_device_id.", 400);
  }
  return value;
}

function optionalBoundedString(value: unknown, label: string, maxBytes = 512) {
  if (value === null || value === undefined || value === "") {
    return null;
  }
  return boundedString(value, label, maxBytes);
}

function boundedInteger(
  value: unknown,
  label: string,
  minimum: number,
  maximum: number
) {
  if (!Number.isInteger(value) || (value as number) < minimum || (value as number) > maximum) {
    throw new HostedWebChatError(`Invalid ${label}.`, 400);
  }
  return value as number;
}
