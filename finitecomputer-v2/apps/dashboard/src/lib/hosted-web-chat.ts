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
  hostedDeviceNewChat,
  hostedDeviceRuntimeCommand,
  hostedDeviceState,
  hostedDeviceUpdates,
  HostedDeviceRequestError,
  type HostedChatAction,
  type HostedChatState,
  type HostedRuntimeCommandResponse,
} from "@/lib/hosted-web-device";

const EMPTY_SCHEMA = "finite.agent.empty.request.v1";
const OWNER_CLAIM = "agent.owner.claim";
const AGENT_BINDING_AUTHORIZATION_REQUIRED =
  "first-time binding bootstrap was not authorized by Project creation";
const AGENT_BINDING_RECOVERY_REQUIRED =
  `canonical Agent conversation requires recovery: ${AGENT_BINDING_AUTHORIZATION_REQUIRED}`;

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
