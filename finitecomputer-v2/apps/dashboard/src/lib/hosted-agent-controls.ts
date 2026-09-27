import { NextResponse } from "next/server";

import { fetchRuntimeAgentNpub } from "@/lib/agent-contact";
import { getAccountAuthContext } from "@/lib/dashboard-auth";
import { loadDashboardMachineAccess } from "@/lib/dashboard-machine-access";
import {
  hostedDeviceAction,
  hostedDeviceConfig,
  hostedDeviceEnsureAgentBinding,
  hostedDeviceOpenAgentBinding,
  hostedDeviceRuntimeCommand,
  hostedDeviceState,
  HostedDeviceRequestError,
  type HostedRuntimeCommand,
  type HostedRuntimeCommandResponse,
} from "@/lib/hosted-web-device";
import type { InferenceRoute } from "@/lib/inference-status";

const EMPTY_SCHEMA = "finite.agent.empty.request.v1";
const OWNER_CLAIM = "agent.owner.claim";
const COMMAND_FAILED_MESSAGE = "The agent could not finish that change. Try again.";
const AGENT_UPDATE_REQUIRED_MESSAGE = "This agent needs an update for this.";
const MODEL_NOT_AVAILABLE_MESSAGE = "That model isn't available on OpenRouter for agents. Pick one from the list.";
const CODEX_VERIFICATION_URI = "https://auth.openai.com/codex/device";
const MAX_TIMESTAMP = 2 ** 53;

export type AgentConnectionsStatus = {
  inference: AgentInferenceStatus;
  simplex?: SimplexConnectionStatus;
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
  /** Absent on agents that predate capability advertisement. */
  capabilities?: string[];
};

/**
 * `profile`, `provider`, and `model` are the legacy fields every agentd sends. The rest is present only
 * when a newer agentd reported it and it passed validation; see `inference-status.ts` for how absence reads.
 */
export type AgentInferenceStatus = {
  profile: string;
  provider: string;
  model: string;
  saved?: { route: InferenceRoute; provider: string | null; model: string | null };
  routes?: {
    finite_private?: {
      state: "configured" | "not_configured" | "unknown";
      reason: "settings_missing" | "credential_missing" | null;
    };
    openrouter?: {
      state: "key_saved" | "no_key" | "unknown";
      key_source: "agent" | "legacy_config" | "environment" | null;
      key_hash: string | null;
      hermes_key: "saved_key" | "other_key" | "none" | "unknown";
      other_pool_keys: "none" | "present" | "unknown";
    };
    openai_codex?: {
      state: "not_signed_in" | "signed_in" | "quota_limited" | "sign_in_required" | "unknown";
      quota_reset_at_ms: number | null;
      reported_quota_reset_at_ms: number | null;
      login: AgentCodexLogin | null;
    };
  };
  fallback?: {
    state: "configured" | "unavailable" | "not_configured" | "off" | "custom" | "unknown";
    reason: "settings_missing" | "credential_missing" | "stale_config" | null;
    model: string | null;
    extra_entries: number;
  };
  operation?: AgentInferenceOperation | null;
};

export type AgentCodexLogin = {
  attempt_id: string;
  state: (typeof CODEX_LOGIN_STATES)[number];
  user_code: string | null;
  verification_uri: string | null;
  expires_at_ms: number;
  poll_interval_s: number;
  error_code: (typeof CODEX_LOGIN_ERRORS)[number] | null;
  retry_after_s: number | null;
};

export type AgentInferenceOperation = {
  id: string;
  kind: "select" | "activate" | "disconnect";
  route: "finite_private" | "openrouter" | "openai_codex";
  model: string | null;
  state: "running" | "succeeded" | "failed";
  phase: (typeof OPERATION_PHASES)[number];
  error_code: (typeof OPERATION_ERRORS)[number] | null;
  attempts: number;
  updated_at_ms: number;
};

/** The agentd reply to select, disconnect, and connect (§3.6, §3.7, §3.9). */
export type InferenceCommandReply =
  | { changed: false }
  | { changed: true; activated: false }
  | { accepted: true; operation_id: string };

/**
 * Response for the inference, OpenRouter connect, and Codex login actions. Older actions still return
 * the status alone. `catalog_checked` is present when an OpenRouter model was saved.
 */
export type AgentConnectionActionResult = {
  status: AgentConnectionsStatus;
  result: InferenceCommandReply | AgentCodexLogin | null;
  catalog_checked?: boolean;
};

type UsageNumbers = { total?: number; daily?: number; weekly?: number; monthly?: number };

/** `agent.openrouter.usage` (§3.8). A missing number stays undefined and is never shown as 0. */
export type OpenRouterUsage = {
  state: "ok" | "no_key" | "key_rejected" | "rate_limited" | "unavailable";
  fetched_at_ms: number | null;
  retry_after_s: number | null;
  other_pool_keys: "none" | "present" | "unknown";
  hermes_key: "saved_key" | "other_key" | "none" | "unknown";
  key: null | {
    key_hash: string;
    limit_usd?: number | null;
    limit_remaining_usd?: number | null;
    limit_reset?: "daily" | "weekly" | "monthly" | "never" | "unknown";
    include_byok_in_limit?: boolean;
    usage_usd: UsageNumbers;
    byok_usage_usd: UsageNumbers;
    is_free_tier?: boolean;
    expires_at?: string | null;
  };
};

/** `agent.codex.models`: the raw live catalog, or why it is unavailable. */
export type CodexModels = {
  state: "live" | "unavailable";
  models: string[];
  reason: "not_signed_in" | "fetch_failed" | "empty" | null;
};

export type PendingSimplexContact = { request_id: string; user_id: string; name: string; age_minutes: number };

export type SimplexConnectionStatus = {
  reset_pending?: boolean;
  pending?: PendingSimplexContact[];
  enabled: boolean;
  ready: boolean;
  address?: string | null;
  qr: string[];
  approved: Array<{ user_id: string; name: string }>;
};

export type AgentConnectionAction =
  | { action: "status" }
  | {
      action: "inference";
      profile: "finite_private" | "openrouter";
      apiKey?: string;
      model?: string;
    }
  | { action: "simplex_connect" | "simplex_reset" }
  | { action: "simplex_approve_request"; request_id: string }
  | { action: "telegram_connect"; token: string }
  | { action: "telegram_approve"; code: string }
  | { action: "telegram_home"; userId: string; name?: string }
  | { action: "telegram_disconnect" }
  | { action: "google_disconnect" }
  | { action: "inference_select"; route: "finite_private" | "openrouter" | "openai_codex"; model?: string }
  | { action: "inference_disconnect"; route: "openrouter" | "openai_codex" }
  | { action: "openrouter_connect_key"; apiKey: string; activate?: { model: string } }
  | { action: "codex_login_start" }
  | { action: "codex_login_cancel"; attemptId: string };

type InferenceConnectionAction = Extract<
  AgentConnectionAction,
  { action: "inference_select" | "inference_disconnect" | "openrouter_connect_key" | "codex_login_start" | "codex_login_cancel" }
>;

export class HostedAgentControlError extends Error {
  constructor(
    message: string,
    readonly status: number,
    /** The agentd error code (§3.2), or a dashboard code such as `agent_update_required`. */
    readonly code: string | null = null
  ) {
    super(message);
  }
}

export async function loadAgentConnections(machineId: string) {
  const context = await hostedAgentContext(machineId);
  await claimOwner(context);
  return statusForContext(context);
}

export async function dispatchAgentConnectionAction(
  machineId: string,
  payload: unknown
): Promise<AgentConnectionsStatus | AgentConnectionActionResult> {
  const action = parseAgentConnectionAction(payload);
  const context = await hostedAgentContext(machineId);
  await claimOwner(context);
  if (isInferenceConnectionAction(action)) {
    return dispatchInferenceAction(context, action);
  }
  if (action.action !== "status") {
    if (action.action === "inference" && action.profile === "openrouter" && action.model) {
      await checkOpenRouterModel(action.model, openRouterCatalog(), async () =>
        savedOpenRouterModel(await statusForContext(context))
      );
    }
    const command = commandForAction(action);
    const secret = action.action === "inference" ? action.apiKey : undefined;
    await sendCommand(context, command.command, command.schema, command.body, secret);
  }
  return statusForContext(context);
}

export async function loadOpenRouterUsage(machineId: string): Promise<OpenRouterUsage> {
  const context = await hostedAgentContext(machineId);
  await claimOwner(context);
  requireCapabilities(await statusForContext(context), ["openrouter.usage.v1"]);
  const response = await sendCommand(context, "agent.openrouter.usage", EMPTY_SCHEMA, {});
  return parseOpenRouterUsage(response.body);
}

export async function loadCodexModels(machineId: string): Promise<CodexModels> {
  const context = await hostedAgentContext(machineId);
  await claimOwner(context);
  requireCapabilities(await statusForContext(context), ["codex.models.v1"]);
  const response = await sendCommand(context, "agent.codex.models", EMPTY_SCHEMA, {});
  return parseCodexModels(response.body);
}

/** Shared by every Connections API route, so each one reports errors the same way. */
export async function connectionsRouteResponse(action: () => Promise<unknown>) {
  try {
    return NextResponse.json(await action(), {
      headers: { "cache-control": "no-store" },
    });
  } catch (error) {
    const known = error instanceof HostedAgentControlError;
    if (!known) {
      console.warn("Connections request failed", {
        error: error instanceof Error ? error.message : String(error),
      });
    }
    return NextResponse.json(
      {
        error: known ? error.message : "Connections are unavailable right now. Try again.",
        code: known ? error.code : null,
      },
      { status: known ? error.status : 500, headers: { "cache-control": "no-store" } }
    );
  }
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
    case "simplex_connect":
    case "simplex_reset":
    case "google_disconnect":
      return { action };
    case "inference": {
      const profile = boundedString(record.profile, "profile", 64);
      if (profile !== "finite_private" && profile !== "openrouter") {
        throw new HostedAgentControlError("Choose Finite Private or OpenRouter.", 400);
      }
      const model = optionalString(record.model, "model", 256);
      return {
        action,
        profile,
        apiKey: optionalString(record.apiKey, "apiKey", 16 * 1024),
        model: profile === "openrouter" && model !== undefined ? openRouterModelName(model) : model,
      };
    }
    case "telegram_connect":
      return { action, token: boundedString(record.token, "token", 256) };
    case "simplex_approve_request": {
      const request_id = boundedString(record.request_id, "request_id", 16);
      if (!/^[a-f0-9]{16}$/.test(request_id)) throw new HostedAgentControlError("Invalid connection request.", 400);
      return { action, request_id };
    }
    case "telegram_approve":
      return { action, code: boundedString(record.code, "code", 16) };
    case "telegram_home":
      return {
        action,
        userId: boundedString(record.userId, "userId", 64),
        name: optionalString(record.name, "name", 128),
      };
    case "inference_select": {
      const route = boundedString(record.route, "route", 64);
      if (route === "finite_private") {
        if (record.model !== undefined && record.model !== null) {
          throw new HostedAgentControlError("model is invalid.", 400);
        }
        return { action, route };
      }
      if (route === "openrouter") {
        return { action, route, model: openRouterModelName(record.model) };
      }
      if (route === "openai_codex") {
        return { action, route, model: codexModelName(record.model) };
      }
      throw new HostedAgentControlError("route is invalid.", 400);
    }
    case "inference_disconnect": {
      const route = boundedString(record.route, "route", 64);
      if (route !== "openrouter" && route !== "openai_codex") {
        throw new HostedAgentControlError("route is invalid.", 400);
      }
      return { action, route };
    }
    case "openrouter_connect_key": {
      const apiKey = boundedString(record.apiKey, "apiKey", 16 * 1024);
      if (record.activate === undefined || record.activate === null) {
        return { action, apiKey };
      }
      return { action, apiKey, activate: { model: openRouterModelName(objectRecord(record.activate).model) } };
    }
    case "codex_login_start":
      return { action };
    case "codex_login_cancel": {
      const attemptId = boundedString(record.attemptId, "attemptId", 36);
      if (!/^cxl_[0-9a-f]{32}$/u.test(attemptId)) throw new HostedAgentControlError("attemptId is invalid.", 400);
      return { action, attemptId };
    }
    default:
      throw new HostedAgentControlError("That connection action is not available.", 400);
  }
}

/** agentd's `validate_model_name`: 1..256 characters, no whitespace or control characters. */
function openRouterModelName(value: unknown) {
  const model = boundedString(value, "model", 256);
  if (/[\s\p{Cc}]/u.test(model)) throw new HostedAgentControlError("model is invalid.", 400);
  return model;
}

function codexModelName(value: unknown) {
  const model = boundedString(value, "model", 128);
  if (!/^[A-Za-z0-9._:-]+$/u.test(model)) throw new HostedAgentControlError("model is invalid.", 400);
  return model;
}

/**
 * The one catalog policy for every OpenRouter model save (§10.4). A catalog member is accepted; with no
 * catalog, any syntax-valid ID is accepted and flagged unchecked; otherwise only the model the agent
 * already has saved, read from status on the server, is accepted. The saved model is read only when
 * it can change the outcome.
 */
export async function checkOpenRouterModel(
  model: string,
  catalog: ReadonlySet<string> | null,
  savedModel: () => Promise<string | null>
): Promise<{ catalogChecked: boolean }> {
  if (!catalog) return { catalogChecked: false };
  if (catalog.has(model) || (await savedModel()) === model) return { catalogChecked: true };
  throw new HostedAgentControlError(MODEL_NOT_AVAILABLE_MESSAGE, 400);
}

/** The saved OpenRouter model, or null when OpenRouter is not the saved route. */
export function savedOpenRouterModel(status: AgentConnectionsStatus) {
  const saved = status.capabilities?.includes("inference.status.v2") ? status.inference.saved : undefined;
  if (saved) return saved.route === "openrouter" ? saved.model : null;
  return status.inference.profile === "openrouter" ? status.inference.model : null;
}

/** PR1 has no server-side OpenRouter catalog, so every syntax-valid model is accepted unchecked. */
function openRouterCatalog(): ReadonlySet<string> | null {
  return null;
}

function isInferenceConnectionAction(action: AgentConnectionAction): action is InferenceConnectionAction {
  return (
    action.action === "inference_select" ||
    action.action === "inference_disconnect" ||
    action.action === "openrouter_connect_key" ||
    action.action === "codex_login_start" ||
    action.action === "codex_login_cancel"
  );
}

async function dispatchInferenceAction(
  context: AgentCommandContext,
  action: InferenceConnectionAction
): Promise<AgentConnectionActionResult> {
  // One status read gates the command on what this agent advertises, so an agent that never
  // advertised a command never receives it, and serves the saved-model check.
  const before = await statusForContext(context);
  requireCapabilities(before, capabilitiesForAction(action));
  const model =
    action.action === "inference_select" && action.route === "openrouter"
      ? action.model
      : action.action === "openrouter_connect_key"
        ? action.activate?.model
        : undefined;
  const modelCheck = model
    ? await checkOpenRouterModel(model, openRouterCatalog(), async () => savedOpenRouterModel(before))
    : undefined;
  const command = inferenceCommandForAction(action);
  const secret = action.action === "openrouter_connect_key" ? action.apiKey : undefined;
  const response = await sendCommand(context, command.command, command.schema, command.body, secret);
  const result =
    action.action === "codex_login_start" || action.action === "codex_login_cancel"
      ? parseCodexLogin(response.body)
      : parseInferenceReply(response.body, action.action === "openrouter_connect_key");
  return {
    status: await statusForContext(context),
    result,
    ...(modelCheck ? { catalog_checked: modelCheck.catalogChecked } : {}),
  };
}

function capabilitiesForAction(action: InferenceConnectionAction) {
  switch (action.action) {
    case "inference_select":
      return action.route === "openai_codex"
        ? ["inference.select.v1", "codex.login.v1"]
        : ["inference.select.v1"];
    case "inference_disconnect":
      return action.route === "openai_codex"
        ? ["inference.disconnect.v1", "codex.login.v1"]
        : ["inference.disconnect.v1"];
    case "openrouter_connect_key":
      return ["openrouter.connect.v1"];
    case "codex_login_start":
    case "codex_login_cancel":
      return ["codex.login.v1"];
  }
}

function requireCapabilities(status: AgentConnectionsStatus, names: string[]) {
  if (!names.every((name) => status.capabilities?.includes(name))) {
    throw new HostedAgentControlError(AGENT_UPDATE_REQUIRED_MESSAGE, 409, "agent_update_required");
  }
}

function inferenceCommandForAction(action: InferenceConnectionAction) {
  switch (action.action) {
    case "inference_select":
      return {
        command: "agent.inference.select",
        schema: "finite.agent.inference.select.v1",
        body: { route: action.route, model: action.model ?? null },
      };
    case "inference_disconnect":
      return {
        command: "agent.inference.disconnect",
        schema: "finite.agent.inference.disconnect.v1",
        body: { route: action.route },
      };
    case "openrouter_connect_key":
      return {
        command: "agent.openrouter.connect",
        schema: "finite.agent.openrouter.connect.v1",
        body: { credential: { kind: "api_key", api_key: action.apiKey }, activate: action.activate ?? null },
      };
    case "codex_login_start":
      return { command: "agent.codex.login.start", schema: "finite.agent.codex.login.start.v1", body: {} };
    case "codex_login_cancel":
      return {
        command: "agent.codex.login.cancel",
        schema: "finite.agent.codex.login.cancel.v1",
        body: { attempt_id: action.attemptId },
      };
  }
}

type AgentCommandContext = {
  account: Awaited<ReturnType<typeof getAccountAuthContext>>;
  config: NonNullable<ReturnType<typeof hostedDeviceConfig>>;
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
  try {
    const bound = await hostedDeviceOpenAgentBinding(config, account, access.coreProject.project.id);
    const binding = bound.hosted_agent_binding;
    if (!binding) {
      throw new HostedAgentControlError("This agent is not available in chat yet.", 503);
    }
    return {
      account,
      config,
      roomId: binding.canonical_room_id,
      targetAccountId: binding.agent_account_id,
    };
  } catch (error) {
    if (!(error instanceof HostedDeviceRequestError) || error.status !== 404) {
      throw error;
    }
  }

  const agentNpub = await fetchRuntimeAgentNpub(access.primaryUrl);
  if (!agentNpub) {
    throw new HostedAgentControlError("This agent is still starting. Try again shortly.", 503);
  }
  const state = await hostedDeviceState(config, account);
  if (!state.status.toLowerCase().includes("running")) {
    await hostedDeviceAction(config, account, { StartRuntime: null });
  }
  const bound = await hostedDeviceEnsureAgentBinding(config, account, {
    project_id: access.coreProject.project.id,
    agent_npub: agentNpub,
    display_name: `Chat with ${access.displayName}`,
  });
  const binding = bound.hosted_agent_binding;
  if (!binding) {
    throw new HostedAgentControlError("This agent is not available in chat yet.", 503);
  }
  return {
    account,
    config,
    roomId: binding.canonical_room_id,
    targetAccountId: binding.agent_account_id,
  };
}

async function claimOwner(context: AgentCommandContext) {
  const response = await hostedDeviceRuntimeCommand(
    context.config,
    context.account,
    agentOwnerClaimCommand(context.roomId, context.targetAccountId)
  );
  assertCommandSucceeded(response);
}

export function agentOwnerClaimCommand(
  roomId: string,
  targetAccountId: string
): HostedRuntimeCommand {
  return {
    room_id: roomId,
    target_account_id: targetAccountId,
    command: OWNER_CLAIM,
    resource_key: "agent.connections",
    schema: EMPTY_SCHEMA,
    body: {},
    reuse_succeeded_owner_claim: true,
    wait_millis: 45_000,
  };
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

/**
 * `secret` is a credential carried in `body`. It goes to the runtime command and nowhere else: an
 * error that repeats it is replaced with generic copy before anything can log or return it.
 */
async function sendCommand(
  context: AgentCommandContext,
  command: string,
  schema: string,
  body: unknown,
  secret?: string
) {
  let response: HostedRuntimeCommandResponse;
  try {
    response = await hostedDeviceRuntimeCommand(context.config, context.account, {
      room_id: context.roomId,
      target_account_id: context.targetAccountId,
      command,
      resource_key: "agent.connections",
      schema,
      body,
      wait_millis: 45_000,
    });
  } catch (error) {
    if (secret && mentionsSecret(error instanceof Error ? error.message : String(error), secret)) {
      throw new HostedAgentControlError(COMMAND_FAILED_MESSAGE, 502);
    }
    throw error;
  }
  assertCommandSucceeded(response, secret);
  return response;
}

function assertCommandSucceeded(response: HostedRuntimeCommandResponse, secret?: string) {
  if (response.status === "succeeded") {
    return;
  }
  const code =
    typeof response.error?.code === "string" && /^[a-z][a-z0-9_]{0,63}$/u.test(response.error.code)
      ? response.error.code
      : null;
  if (code === "unsupported_command") {
    throw new HostedAgentControlError(AGENT_UPDATE_REQUIRED_MESSAGE, 409, "agent_update_required");
  }
  const message = typeof response.error?.message === "string" ? response.error.message : "";
  throw new HostedAgentControlError(
    message && !(secret && mentionsSecret(message, secret)) ? message : COMMAND_FAILED_MESSAGE,
    code === "unauthorized" ? 403 : 502,
    code
  );
}

function mentionsSecret(text: string, secret: string) {
  return text.includes(secret) || text.includes(JSON.stringify(secret).slice(1, -1));
}

function commandForAction(
  action: Exclude<AgentConnectionAction, { action: "status" } | InferenceConnectionAction>
) {
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
    case "simplex_connect":
      return { command: "agent.simplex.connect", schema: EMPTY_SCHEMA, body: {} };
    case "simplex_reset":
      return { command: "agent.simplex.reset", schema: "finite.agent.simplex.reset.v1", body: {} };
    case "simplex_approve_request":
      return { command: "agent.simplex.approve_request", schema: "finite.agent.simplex.approve-request.v1", body: { request_id: action.request_id } };
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

export function parseConnectionsStatus(value: unknown): AgentConnectionsStatus {
  const root = objectRecord(value);
  const inference = objectRecord(root.inference);
  const telegram = objectRecord(root.telegram);
  const google = objectRecord(root.google);
  const capabilities = parseCapabilities(root.capabilities);
  return {
    inference: {
      // An unknown legacy profile is kept, not thrown: the view reads the raw provider instead.
      profile: boundedString(inference.profile, "inference profile", 64),
      provider: boundedString(inference.provider, "inference provider", 128),
      model: boundedString(inference.model, "inference model", 256),
      ...parseInferenceV2(inference),
    },
    ...(capabilities ? { capabilities } : {}),
    simplex: root.simplex == null ? undefined : parseSimplexStatus(root.simplex),
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

const CODEX_LOGIN_STATES = ["pending", "committing", "approved", "canceled", "expired", "failed", "interrupted"] as const;
const CODEX_LOGIN_ERRORS = ["rate_limited", "start_failed", "poll_failed", "exchange_failed", "network_error", "save_failed"] as const;
const OPERATION_PHASES = [
  "accepted", "config_written", "restarting", "verifying",
  "login_cancelled", "route_switched", "credential_removed", "cleanup",
] as const;
const OPERATION_ERRORS = ["config_invalid", "config_conflict", "supervisor_unavailable", "helper_unavailable", "verify_failed"] as const;

// Everything below parses fields a newer agentd adds (§10.3). Runtime data is untrusted: an invalid
// optional value degrades to null or "unknown", an invalid required value drops its object, and
// nothing here throws, so one bad field can't blank the rest of Connections.

function parseCapabilities(value: unknown) {
  if (
    !Array.isArray(value) ||
    value.length > 32 ||
    !value.every((entry) => typeof entry === "string" && /^[a-z0-9._]{1,64}$/u.test(entry))
  ) {
    return undefined;
  }
  return value as string[];
}

function parseInferenceV2(inference: Record<string, unknown>) {
  const saved = parseSaved(inference.saved);
  const routes = recordOrNull(inference.routes);
  const fallback = parseFallback(inference.fallback);
  const operation = inference.operation === null ? null : parseOperation(inference.operation);
  return {
    ...(saved ? { saved } : {}),
    ...(routes ? { routes: parseRoutes(routes) } : {}),
    ...(fallback ? { fallback } : {}),
    ...(operation !== undefined ? { operation } : {}),
  } satisfies Partial<AgentInferenceStatus>;
}

function parseSaved(value: unknown): AgentInferenceStatus["saved"] {
  const saved = recordOrNull(value);
  const route = saved && enumOrUndefined(saved.route, ["finite_private", "openrouter", "openai_codex", "other"] as const);
  if (!saved || !route) return undefined;
  return { route, provider: textOrNull(saved.provider, 128), model: textOrNull(saved.model, 256) };
}

function parseRoutes(routes: Record<string, unknown>): NonNullable<AgentInferenceStatus["routes"]> {
  const finitePrivate = recordOrNull(routes.finite_private);
  const openrouter = recordOrNull(routes.openrouter);
  const codex = recordOrNull(routes.openai_codex);
  return {
    ...(finitePrivate
      ? {
          finite_private: {
            state: enumOrUnknown(finitePrivate.state, ["configured", "not_configured"] as const),
            reason: enumOrUndefined(finitePrivate.reason, ["settings_missing", "credential_missing"] as const) ?? null,
          },
        }
      : {}),
    ...(openrouter
      ? {
          openrouter: {
            state: enumOrUnknown(openrouter.state, ["key_saved", "no_key"] as const),
            key_source: enumOrUndefined(openrouter.key_source, ["agent", "legacy_config", "environment"] as const) ?? null,
            key_hash: keyHashOrNull(openrouter.key_hash),
            hermes_key: enumOrUnknown(openrouter.hermes_key, ["saved_key", "other_key", "none"] as const),
            other_pool_keys: enumOrUnknown(openrouter.other_pool_keys, ["none", "present"] as const),
          },
        }
      : {}),
    ...(codex
      ? {
          openai_codex: {
            state: enumOrUnknown(codex.state, ["not_signed_in", "signed_in", "quota_limited", "sign_in_required"] as const),
            quota_reset_at_ms: timestampOrUndefined(codex.quota_reset_at_ms) ?? null,
            reported_quota_reset_at_ms: timestampOrUndefined(codex.reported_quota_reset_at_ms) ?? null,
            login: parseCodexLogin(codex.login),
          },
        }
      : {}),
  };
}

function parseFallback(value: unknown): AgentInferenceStatus["fallback"] {
  const fallback = recordOrNull(value);
  if (!fallback) return undefined;
  return {
    state: enumOrUnknown(fallback.state, ["configured", "unavailable", "not_configured", "off", "custom"] as const),
    reason: enumOrUndefined(fallback.reason, ["settings_missing", "credential_missing", "stale_config"] as const) ?? null,
    model: textOrNull(fallback.model, 256),
    extra_entries: integerInRange(fallback.extra_entries, 0, 16) ?? 0,
  };
}

function parseOperation(value: unknown): AgentInferenceOperation | undefined {
  const operation = recordOrNull(value);
  if (!operation) return undefined;
  const id = typeof operation.id === "string" && /^op_[0-9a-f]{32}$/u.test(operation.id) ? operation.id : undefined;
  const kind = enumOrUndefined(operation.kind, ["select", "activate", "disconnect"] as const);
  const route = enumOrUndefined(operation.route, ["finite_private", "openrouter", "openai_codex"] as const);
  const state = enumOrUndefined(operation.state, ["running", "succeeded", "failed"] as const);
  const phase = enumOrUndefined(operation.phase, OPERATION_PHASES);
  const attempts = integerInRange(operation.attempts, 0, 10);
  const updatedAtMs = timestampOrUndefined(operation.updated_at_ms);
  if (!id || !kind || !route || !state || !phase || attempts === undefined || updatedAtMs === undefined) {
    return undefined;
  }
  return {
    id,
    kind,
    route,
    model: textOrNull(operation.model, 256),
    state,
    phase,
    error_code: enumOrUndefined(operation.error_code, OPERATION_ERRORS) ?? null,
    attempts,
    updated_at_ms: updatedAtMs,
  };
}

/** Also parses the `codex_login_start` and `codex_login_cancel` replies. */
function parseCodexLogin(value: unknown): AgentCodexLogin | null {
  const login = recordOrNull(value);
  if (!login) return null;
  const attemptId =
    typeof login.attempt_id === "string" && /^cxl_[0-9a-f]{32}$/u.test(login.attempt_id) ? login.attempt_id : undefined;
  const state = enumOrUndefined(login.state, CODEX_LOGIN_STATES);
  const expiresAtMs = timestampOrUndefined(login.expires_at_ms);
  const pollIntervalS = integerInRange(login.poll_interval_s, 1, 60);
  if (!attemptId || !state || expiresAtMs === undefined || pollIntervalS === undefined) return null;
  const parsed: AgentCodexLogin = {
    attempt_id: attemptId,
    state,
    user_code:
      typeof login.user_code === "string" && /^[A-Z0-9-]{4,16}$/iu.test(login.user_code) ? login.user_code : null,
    verification_uri: login.verification_uri === CODEX_VERIFICATION_URI ? CODEX_VERIFICATION_URI : null,
    expires_at_ms: expiresAtMs,
    poll_interval_s: pollIntervalS,
    error_code: enumOrUndefined(login.error_code, CODEX_LOGIN_ERRORS) ?? null,
    retry_after_s: integerInRange(login.retry_after_s, 0, 3600) ?? null,
  };
  if (login.verification_uri !== null && login.verification_uri !== CODEX_VERIFICATION_URI) {
    // Never show a sign-in link the agent made up.
    return { ...parsed, state: "failed", user_code: null, error_code: "start_failed" };
  }
  return parsed;
}

function parseInferenceReply(value: unknown, connect: boolean): InferenceCommandReply | null {
  const reply = recordOrNull(value);
  if (!reply) return null;
  if (reply.accepted === true && typeof reply.operation_id === "string" && /^op_[0-9a-f]{32}$/u.test(reply.operation_id)) {
    return { accepted: true, operation_id: reply.operation_id };
  }
  if (reply.changed === false) return { changed: false };
  if (connect && reply.changed === true && reply.activated === false) return { changed: true, activated: false };
  return null;
}

export function parseOpenRouterUsage(value: unknown): OpenRouterUsage {
  const usage = recordOrNull(value);
  const state = enumOrUndefined(usage?.state, ["ok", "no_key", "key_rejected", "rate_limited", "unavailable"] as const);
  const key = state === "ok" ? parseUsageKey(usage?.key) : null;
  return {
    state: !state || (state === "ok" && !key) ? "unavailable" : state,
    fetched_at_ms: timestampOrUndefined(usage?.fetched_at_ms) ?? null,
    retry_after_s: integerInRange(usage?.retry_after_s, 0, 3600) ?? null,
    other_pool_keys: enumOrUnknown(usage?.other_pool_keys, ["none", "present"] as const),
    hermes_key: enumOrUnknown(usage?.hermes_key, ["saved_key", "other_key", "none"] as const),
    key,
  };
}

function parseUsageKey(value: unknown): OpenRouterUsage["key"] {
  const key = recordOrNull(value);
  const keyHash = keyHashOrNull(key?.key_hash);
  if (!key || !keyHash) return null;
  const parsed: NonNullable<OpenRouterUsage["key"]> = {
    key_hash: keyHash,
    usage_usd: parseUsageNumbers(key.usage_usd),
    byok_usage_usd: parseUsageNumbers(key.byok_usage_usd),
  };
  if (key.limit_usd === null || finiteNumber(key.limit_usd)) parsed.limit_usd = key.limit_usd;
  if (key.limit_remaining_usd === null || finiteNumber(key.limit_remaining_usd)) {
    parsed.limit_remaining_usd = key.limit_remaining_usd;
  }
  if (key.limit_reset !== undefined) {
    parsed.limit_reset = enumOrUnknown(key.limit_reset, ["daily", "weekly", "monthly", "never"] as const);
  }
  if (typeof key.include_byok_in_limit === "boolean") parsed.include_byok_in_limit = key.include_byok_in_limit;
  if (typeof key.is_free_tier === "boolean") parsed.is_free_tier = key.is_free_tier;
  if (key.expires_at === null) {
    parsed.expires_at = null;
  } else if (
    typeof key.expires_at === "string" &&
    key.expires_at.length <= 40 &&
    !Number.isNaN(Date.parse(key.expires_at))
  ) {
    parsed.expires_at = key.expires_at;
  }
  return parsed;
}

function parseUsageNumbers(value: unknown): UsageNumbers {
  const numbers = recordOrNull(value);
  const parsed: UsageNumbers = {};
  for (const period of ["total", "daily", "weekly", "monthly"] as const) {
    const amount = numbers?.[period];
    if (finiteNumber(amount)) parsed[period] = amount;
  }
  return parsed;
}

export function parseCodexModels(value: unknown): CodexModels {
  const reply = recordOrNull(value);
  const reason = enumOrUndefined(reply?.reason, ["not_signed_in", "fetch_failed", "empty"] as const) ?? null;
  if (reply?.state !== "live" || !Array.isArray(reply.models)) {
    return { state: "unavailable", models: [], reason };
  }
  const models = reply.models.filter(
    (id): id is string => typeof id === "string" && /^[A-Za-z0-9._:-]{1,128}$/u.test(id)
  );
  return { state: "live", models: [...new Set(models)].slice(0, 200), reason: null };
}

function recordOrNull(value: unknown): Record<string, unknown> | null {
  return value && typeof value === "object" && !Array.isArray(value) ? (value as Record<string, unknown>) : null;
}

function enumOrUndefined<T extends string>(value: unknown, values: readonly T[]): T | undefined {
  return typeof value === "string" && (values as readonly string[]).includes(value) ? (value as T) : undefined;
}

function enumOrUnknown<T extends string>(value: unknown, values: readonly T[]): T | "unknown" {
  return enumOrUndefined(value, values) ?? "unknown";
}

function textOrNull(value: unknown, max: number) {
  return typeof value === "string" && value.trim() && value.length <= max ? value.trim() : null;
}

function keyHashOrNull(value: unknown) {
  return typeof value === "string" && /^[0-9a-f]{64}$/u.test(value) ? value : null;
}

function finiteNumber(value: unknown): value is number {
  return typeof value === "number" && Number.isFinite(value);
}

function integerInRange(value: unknown, min: number, max: number) {
  return typeof value === "number" && Number.isInteger(value) && value >= min && value <= max ? value : undefined;
}

function timestampOrUndefined(value: unknown) {
  return typeof value === "number" && Number.isInteger(value) && value > 0 && value < MAX_TIMESTAMP ? value : undefined;
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

export function parseSimplexStatus(value: unknown): SimplexConnectionStatus {
  const status = objectRecord(value);
  const address = optionalString(status.address, "SimpleX address", 4096);
  if (address) {
    const url = new URL(address);
    if (url.protocol !== "https:" && url.protocol !== "simplex:") {
      throw new HostedAgentControlError("The agent returned an invalid SimpleX address.", 502);
    }
  }
  const qr = status.qr;
  if (!Array.isArray(qr) || qr.length > 177 || qr.some(row => typeof row !== "string" || row.length !== qr.length || !/^[01]+$/u.test(row))) {
    throw new HostedAgentControlError("The agent returned an invalid SimpleX QR code.", 502);
  }
  return {
    reset_pending: status.reset_pending === true,
    enabled: status.enabled === true,
    ready: status.ready === true,
    address,
    qr,
    approved: parsePeople(status.approved),
    pending: status.pending === undefined ? undefined : parseSimplexPending(status.pending),
  };
}

function parseSimplexPending(value: unknown): PendingSimplexContact[] {
  if (!Array.isArray(value)) throw new HostedAgentControlError("Invalid SimpleX requests.", 502);
  return value.slice(0, 32).map(entry => {
    const row = objectRecord(entry);
    const request_id = boundedString(row.request_id, "request_id", 16);
    if (!/^[a-f0-9]{16}$/.test(request_id) || typeof row.age_minutes !== "number" || !Number.isInteger(row.age_minutes) || row.age_minutes < 0) {
      throw new HostedAgentControlError("Invalid SimpleX request.", 502);
    }
    return { request_id, user_id: boundedString(row.user_id, "contact ID", 64), name: optionalString(row.name, "contact name", 128) ?? "", age_minutes: row.age_minutes };
  });
}
