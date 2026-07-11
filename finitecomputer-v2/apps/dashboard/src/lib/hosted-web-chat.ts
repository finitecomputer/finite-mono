import { fetchRuntimeAgentNpub } from "@/lib/agent-contact";
import { CHAT_UNAVAILABLE_MESSAGE } from "@/lib/chat-product-copy";
import { getAccountAuthContext } from "@/lib/dashboard-auth";
import { loadDashboardMachineAccess } from "@/lib/dashboard-machine-access";
import {
  hostedDeviceAction,
  hostedDeviceAttachment,
  hostedDeviceAttachments,
  hostedDeviceConfig,
  hostedDeviceRuntimeCommand,
  hostedDeviceState,
  hostedDeviceUpdates,
  type HostedChatAction,
  type HostedChatProfile,
  type HostedChatState,
  type HostedRuntimeCommandResponse,
} from "@/lib/hosted-web-device";
import {
  trustedOwnerClaims,
  type TrustedOwnerClaimScope,
} from "@/lib/trusted-owner-claim";

const EMPTY_SCHEMA = "finite.agent.empty.request.v1";
const OWNER_CLAIM = "agent.owner.claim";

export class HostedWebChatError extends Error {
  constructor(
    message: string,
    readonly status: number
  ) {
    super(message);
  }
}

export function hostedWebChatErrorMessage(error: unknown) {
  return error instanceof HostedWebChatError ? error.message : CHAT_UNAVAILABLE_MESSAGE;
}

export async function bootstrapHostedWebChat(machineId: string) {
  const context = await hostedWebChatContext(machineId);
  let state = await hostedDeviceState(context.config, context.account);
  state = await ensureRuntimeStarted(context, state);

  const agentNpub = await fetchRuntimeAgentNpub(context.primaryUrl);
  if (!agentNpub) {
    throw new HostedWebChatError("Your agent is still getting ready. Try again shortly.", 503);
  }
  state = await connectAgentProfile(context, state, agentNpub);
  await claimAgentOwner(context, state, agentNpub, machineId);

  return state;
}

export async function dispatchHostedWebChatAction(machineId: string, payload: unknown) {
  const context = await hostedWebChatContext(machineId);
  const action = parseHostedChatAction(payload);
  return hostedDeviceAction(context.config, context.account, action);
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

async function hostedWebChatContext(machineId: string) {
  const account = await getAccountAuthContext();
  if (!account.workosUserId || !account.emailVerified) {
    throw new HostedWebChatError("Sign in again to use chat.", 401);
  }
  const access = await loadDashboardMachineAccess(machineId, { coreCacheMode: "swr" });
  if (!access) {
    throw new HostedWebChatError("Agent not found.", 404);
  }
  const config = hostedDeviceConfig();
  if (!config) {
    throw new HostedWebChatError(CHAT_UNAVAILABLE_MESSAGE, 503);
  }
  return {
    account,
    config,
    primaryUrl: access.primaryUrl,
    agentName: access.displayName,
  };
}

async function connectAgentProfile(
  context: Awaited<ReturnType<typeof hostedWebChatContext>>,
  state: HostedChatState,
  agentNpub: string
) {
  state = await hostedDeviceAction(context.config, context.account, {
    ScanTarget: { value: agentNpub },
  });
  const profile = profileForNpub(state, agentNpub);
  if (!profile) {
    return state;
  }
  return hostedDeviceAction(context.config, context.account, {
    StartProfileChat: {
      profile,
      display_name: `Chat with ${context.agentName}`,
    },
  });
}

async function claimAgentOwner(
  context: Awaited<ReturnType<typeof hostedWebChatContext>>,
  state: HostedChatState,
  agentNpub: string,
  machineId: string
) {
  const profile = profileForNpub(state, agentNpub);
  const roomId = state.selected_room_id?.trim();
  if (!profile || !roomId) {
    throw new HostedWebChatError("Your chat is still getting ready. Try again shortly.", 503);
  }
  const scope = ownerClaimScope(context, state, machineId, roomId, profile.account_id);
  if (trustedOwnerClaims.established(state, scope)) {
    return;
  }
  const response = await hostedDeviceRuntimeCommand(context.config, context.account, {
    room_id: roomId,
    target_account_id: profile.account_id,
    command: OWNER_CLAIM,
    resource_key: "agent.connections",
    schema: EMPTY_SCHEMA,
    body: {},
    wait_millis: 45_000,
  });
  assertCommandSucceeded(response);
  trustedOwnerClaims.remember(scope);
}

function ownerClaimScope(
  context: Awaited<ReturnType<typeof hostedWebChatContext>>,
  state: HostedChatState,
  machineId: string,
  roomId: string,
  agentAccountId: string
): TrustedOwnerClaimScope {
  return {
    workosUserId: context.account.workosUserId!,
    machineId,
    hostedAccountId: state.identity.account_id,
    roomId,
    agentAccountId,
  };
}

function profileForNpub(state: HostedChatState, npub: string): HostedChatProfile | null {
  return state.profiles.find((profile) => profile.npub.toLowerCase() === npub.toLowerCase()) ?? null;
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
    case "StartTopicChat": {
      const value = objectRecord(input, operation);
      return {
        StartTopicChat: {
          room_id: boundedString(value.room_id, "room_id"),
          topic_id: boundedString(value.topic_id, "topic_id"),
          reason: optionalBoundedString(value.reason, "reason", 256),
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
