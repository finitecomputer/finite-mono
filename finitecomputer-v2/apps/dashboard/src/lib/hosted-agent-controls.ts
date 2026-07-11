import { fetchRuntimeAgentNpub } from "@/lib/agent-contact";
import { getAccountAuthContext } from "@/lib/dashboard-auth";
import { loadDashboardMachineAccess } from "@/lib/dashboard-machine-access";
import {
  hostedDeviceAction,
  hostedDeviceConfig,
  hostedDeviceRuntimeCommand,
  hostedDeviceState,
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

export type AgentConnectionsStatus = {
  inference: {
    profile: "finite_private" | "openrouter";
    provider: string;
    model: string;
  };
  telegram: {
    connected: boolean;
    home_channel?: string | null;
    pending: Array<{ user_id: string; name: string }>;
    approved: Array<{ user_id: string; name: string }>;
  };
  google: {
    connected: boolean;
    email?: string | null;
  };
};

export type AgentConnectionAction =
  | { action: "status" }
  | {
      action: "inference";
      profile: "finite_private" | "openrouter";
      apiKey?: string;
      model?: string;
    }
  | { action: "telegram_connect"; token: string }
  | { action: "telegram_approve"; code: string }
  | { action: "telegram_home"; userId: string; name?: string }
  | { action: "telegram_disconnect" }
  | { action: "google_disconnect" };

export class HostedAgentControlError extends Error {
  constructor(
    message: string,
    readonly status: number
  ) {
    super(message);
  }
}

export async function loadAgentConnections(machineId: string) {
  const context = await hostedAgentContext(machineId);
  await claimOwner(context);
  return statusForContext(context);
}

export async function dispatchAgentConnectionAction(machineId: string, payload: unknown) {
  const action = parseAgentConnectionAction(payload);
  const context = await hostedAgentContext(machineId);
  await claimOwner(context);
  if (action.action !== "status") {
    const command = commandForAction(action);
    await sendCommand(context, command.command, command.schema, command.body);
  }
  return statusForContext(context);
}

export async function applyGoogleConnection(
  machineId: string,
  body: {
    clientId: string;
    clientSecret: string;
    refreshToken: string;
    accessToken: string;
    redirectUri: string;
    connectedEmail: string;
    scopes: string[];
  }
) {
  const context = await hostedAgentContext(machineId);
  await claimOwner(context);
  await sendCommand(context, "agent.google.apply", "finite.agent.google.apply.v1", {
    client_id: body.clientId,
    client_secret: body.clientSecret,
    refresh_token: body.refreshToken,
    access_token: body.accessToken,
    redirect_uri: body.redirectUri,
    connected_email: body.connectedEmail,
    scopes: body.scopes,
  });
  return statusForContext(context);
}

export function parseAgentConnectionAction(payload: unknown): AgentConnectionAction {
  const record = objectRecord(payload);
  const action = boundedString(record.action, "action", 64);
  switch (action) {
    case "status":
    case "telegram_disconnect":
    case "google_disconnect":
      return { action };
    case "inference": {
      const profile = boundedString(record.profile, "profile", 64);
      if (profile !== "finite_private" && profile !== "openrouter") {
        throw new HostedAgentControlError("Choose Finite Private or OpenRouter.", 400);
      }
      return {
        action,
        profile,
        apiKey: optionalString(record.apiKey, "apiKey", 16 * 1024),
        model: optionalString(record.model, "model", 256),
      };
    }
    case "telegram_connect":
      return { action, token: boundedString(record.token, "token", 256) };
    case "telegram_approve":
      return { action, code: boundedString(record.code, "code", 16) };
    case "telegram_home":
      return {
        action,
        userId: boundedString(record.userId, "userId", 64),
        name: optionalString(record.name, "name", 128),
      };
    default:
      throw new HostedAgentControlError("That connection action is not available.", 400);
  }
}

type AgentCommandContext = {
  account: Awaited<ReturnType<typeof getAccountAuthContext>>;
  config: NonNullable<ReturnType<typeof hostedDeviceConfig>>;
  state: HostedChatState;
  claimScope: TrustedOwnerClaimScope;
  roomId: string;
  targetAccountId: string;
};

async function hostedAgentContext(machineId: string): Promise<AgentCommandContext> {
  const account = await getAccountAuthContext();
  if (!account.workosUserId || !account.emailVerified) {
    throw new HostedAgentControlError("Sign in again to manage this agent.", 401);
  }
  const access = await loadDashboardMachineAccess(machineId, { coreCacheMode: "swr" });
  if (!access) {
    throw new HostedAgentControlError("Agent not found.", 404);
  }
  const config = hostedDeviceConfig();
  if (!config) {
    throw new HostedAgentControlError("Connections are unavailable right now.", 503);
  }
  const agentNpub = await fetchRuntimeAgentNpub(access.primaryUrl);
  if (!agentNpub) {
    throw new HostedAgentControlError("This agent is still starting. Try again shortly.", 503);
  }

  let state = await hostedDeviceState(config, account);
  if (!state.status.toLowerCase().includes("running")) {
    state = await hostedDeviceAction(config, account, { StartRuntime: null });
  }
  state = await hostedDeviceAction(config, account, { ScanTarget: { value: agentNpub } });
  const profile = profileForNpub(state, agentNpub);
  if (!profile) {
    throw new HostedAgentControlError("This agent is not available in chat yet.", 503);
  }
  state = await hostedDeviceAction(config, account, {
    StartProfileChat: { profile, display_name: `Chat with ${access.displayName}` },
  });
  const roomId = state.selected_room_id?.trim();
  if (!roomId) {
    throw new HostedAgentControlError("This agent is not available in chat yet.", 503);
  }
  return {
    account,
    config,
    state,
    claimScope: {
      workosUserId: account.workosUserId,
      machineId,
      hostedAccountId: state.identity.account_id,
      roomId,
      agentAccountId: profile.account_id,
    },
    roomId,
    targetAccountId: profile.account_id,
  };
}

function profileForNpub(state: HostedChatState, npub: string): HostedChatProfile | null {
  return (
    state.profiles.find((profile) => profile.npub.toLowerCase() === npub.toLowerCase()) ?? null
  );
}

async function claimOwner(context: AgentCommandContext) {
  if (trustedOwnerClaims.established(context.state, context.claimScope)) {
    return;
  }
  await sendCommand(context, OWNER_CLAIM, EMPTY_SCHEMA, {});
  trustedOwnerClaims.remember(context.claimScope);
}

async function statusForContext(context: AgentCommandContext) {
  const response = await sendCommand(
    context,
    "agent.connections.status",
    EMPTY_SCHEMA,
    {}
  );
  return parseConnectionsStatus(response.body);
}

async function sendCommand(
  context: AgentCommandContext,
  command: string,
  schema: string,
  body: unknown
) {
  const response = await hostedDeviceRuntimeCommand(context.config, context.account, {
    room_id: context.roomId,
    target_account_id: context.targetAccountId,
    command,
    resource_key: "agent.connections",
    schema,
    body,
    wait_millis: 45_000,
  });
  assertCommandSucceeded(response);
  return response;
}

function assertCommandSucceeded(response: HostedRuntimeCommandResponse) {
  if (response.status === "succeeded") {
    return;
  }
  throw new HostedAgentControlError(
    response.error?.message || "The agent could not finish that change. Try again.",
    response.error?.code === "unauthorized" ? 403 : 502
  );
}

function commandForAction(action: Exclude<AgentConnectionAction, { action: "status" }>) {
  switch (action.action) {
    case "inference":
      return {
        command: "agent.inference.apply",
        schema: "finite.agent.inference.apply.v1",
        body: {
          profile: action.profile,
          api_key: action.apiKey,
          model: action.model,
        },
      };
    case "telegram_connect":
      return {
        command: "agent.telegram.connect",
        schema: "finite.agent.telegram.connect.v1",
        body: { token: action.token },
      };
    case "telegram_approve":
      return {
        command: "agent.telegram.approve",
        schema: "finite.agent.telegram.approve.v1",
        body: { code: action.code },
      };
    case "telegram_home":
      return {
        command: "agent.telegram.home",
        schema: "finite.agent.telegram.home.v1",
        body: { user_id: action.userId, name: action.name },
      };
    case "telegram_disconnect":
      return { command: "agent.telegram.disconnect", schema: EMPTY_SCHEMA, body: {} };
    case "google_disconnect":
      return { command: "agent.google.disconnect", schema: EMPTY_SCHEMA, body: {} };
  }
}

function parseConnectionsStatus(value: unknown): AgentConnectionsStatus {
  const root = objectRecord(value);
  const inference = objectRecord(root.inference);
  const telegram = objectRecord(root.telegram);
  const google = objectRecord(root.google);
  const profile = boundedString(inference.profile, "inference profile", 64);
  if (profile !== "finite_private" && profile !== "openrouter") {
    throw new HostedAgentControlError("The agent returned an unknown inference choice.", 502);
  }
  return {
    inference: {
      profile,
      provider: boundedString(inference.provider, "inference provider", 128),
      model: boundedString(inference.model, "inference model", 256),
    },
    telegram: {
      connected: telegram.connected === true,
      home_channel: optionalString(telegram.home_channel, "Telegram chat", 256),
      pending: parsePeople(telegram.pending),
      approved: parsePeople(telegram.approved),
    },
    google: {
      connected: google.connected === true,
      email: optionalString(google.email, "Google email", 320),
    },
  };
}

function parsePeople(value: unknown) {
  if (!Array.isArray(value)) {
    return [];
  }
  return value.slice(0, 32).map((entry) => {
    const person = objectRecord(entry);
    return {
      user_id: boundedString(person.user_id, "Telegram user", 64),
      name: optionalString(person.name, "Telegram name", 128) ?? "",
    };
  });
}

function objectRecord(value: unknown): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new HostedAgentControlError("The connection request is invalid.", 400);
  }
  return value as Record<string, unknown>;
}

function boundedString(value: unknown, field: string, max: number) {
  if (typeof value !== "string" || !value.trim() || value.length > max) {
    throw new HostedAgentControlError(`${field} is invalid.`, 400);
  }
  return value.trim();
}

function optionalString(value: unknown, field: string, max: number) {
  if (value === undefined || value === null || value === "") {
    return undefined;
  }
  return boundedString(value, field, max);
}
