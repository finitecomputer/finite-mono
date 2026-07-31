import type { AccountAuthContext } from "@/lib/dashboard-auth";

const HOSTED_DEVICE_TIMEOUT_MS = 15_000;

export class HostedDeviceRequestError extends Error {
  constructor(
    message: string,
    readonly status: number
  ) {
    super(message);
    this.name = "HostedDeviceRequestError";
  }
}

export type HostedChatRoom = {
  room_id: string;
  display_name: string;
  state: "Connected" | "WaitingForApproval" | "Joining" | "UnavailableOnDevice";
  status: string;
  user_status_text: string;
  last_message_preview: string;
  unread_count: number;
  can_load_older: boolean;
  is_agent_chat: boolean;
};

export type HostedChatSummary = {
  chat_id: string;
  title: string;
  last_message_preview: string;
  unread_count: number;
  message_count: number;
  started_seq: number;
  updated_seq: number;
  active: boolean;
  archived: boolean;
};

export type HostedChatTopic = {
  room_id: string;
  topic_id: string;
  title: string;
  description?: string | null;
  last_message_preview: string;
  unread_count: number;
  message_count: number;
  created_seq: number;
  updated_seq: number;
  archived: boolean;
  active_chat_id?: string | null;
  chats: HostedChatSummary[];
};

export type HostedChatMediaKind = "Image" | "VoiceNote" | "Video" | "File";

export type HostedChatMediaAttachment = {
  attachment_id: string;
  url?: string | null;
  mime_type: string;
  filename: string;
  kind: HostedChatMediaKind;
  width?: number | null;
  height?: number | null;
  upload_progress_per_mille?: number | null;
  download_progress_per_mille?: number | null;
};

export type HostedChatOutboundDelivery = {
  local_send: "Sending" | "Sent";
  server_delivery: "Undelivered" | "Delivered" | { Failed: { reason: string } };
};

export type HostedChatMessage = {
  room_id: string;
  seq: number;
  message_id: string;
  conversation_id?: string | null;
  chat_id?: string | null;
  sender_account_id: string;
  sender_device_id: string;
  sender_display_name: string;
  sender_npub?: string | null;
  text: string;
  display_content: string;
  rich_text_json?: string;
  reply_to_message_id?: string | null;
  is_mine: boolean;
  outbound_delivery?: HostedChatOutboundDelivery | null;
  media: HostedChatMediaAttachment[];
  kind: "message" | "status" | "tool" | "media" | string;
  status: "running" | "complete" | string;
  final_delivery: boolean;
  edit_of_message_id?: string | null;
  clarification?: {
    state: "requested" | "answered";
    request_id: string;
    turn_id: string;
    prompt: string;
    choices: string[];
    expires_at_unix_seconds: number;
    answer_message_id?: string | null;
  } | null;
  timestamp_unix_seconds: number;
  display_timestamp: string;
};

export type HostedChatTypingMember = {
  room_id: string;
  topic_id?: string | null;
  chat_id?: string | null;
  account_id: string;
  device_id: string;
  display_name: string;
  picture?: string | null;
  npub?: string | null;
  activity_kind: "typing" | "thinking" | "working" | string;
};

export type HostedChatProfile = {
  account_id: string;
  npub: string;
  display_name: string;
  about?: string | null;
  picture?: string | null;
  stale: boolean;
  is_agent: boolean;
};

export type HostedChatDevice = {
  account_id: string;
  device_id: string;
  active: boolean;
  current_device: boolean;
  revoked: boolean;
  room_count: number;
};

export type HostedChatState = {
  rev: number;
  identity: {
    account_id: string;
    device_id: string;
  };
  rooms: HostedChatRoom[];
  selected_room_id?: string | null;
  topics: HostedChatTopic[];
  selected_topic_id?: string | null;
  selected_chat_id?: string | null;
  active_profile_id?: string | null;
  status: string;
  toast?: string | null;
  messages: HostedChatMessage[];
  profiles: HostedChatProfile[];
  devices: HostedChatDevice[];
  typing_members: HostedChatTypingMember[];
  hosted_agent_binding?: HostedAgentBinding | null;
  flow: {
    notice_text?: string | null;
    notice_busy: boolean;
    scan_in_flight: boolean;
    scan_result: string;
    image_upload_url?: string | null;
  };
};

export type HostedAgentBinding = {
  version: number;
  project_id: string;
  human_account_id: string;
  agent_account_id: string;
  agent_npub: string;
  canonical_room_id: string;
  associated_room_ids: string[];
};

export type HostedAgentBindingBootstrapAuthorization = {
  status: "authorized" | "already_authorized" | "already_bound";
};

export type HostedChatAction =
  | { StartRuntime: null }
  | { OpenRoom: { room_id: string } }
  | { OpenTopic: { room_id: string; topic_id: string } }
  | { OpenChat: { room_id: string; topic_id: string; chat_id: string } }
  | { CreateTopic: { room_id: string; title: string } }
  | {
      StartTopicChat: {
        room_id: string;
        topic_id: string;
        reason?: string | null;
      };
    }
  | {
      StartTopicChatIntent: {
        room_id: string;
        topic_id: string;
        reason?: string | null;
        intent_key: string;
      };
    }
  | {
      RenameChat: {
        room_id: string;
        topic_id: string;
        chat_id: string;
        title: string;
      };
    }
  | {
      SetChatArchived: {
        room_id: string;
        topic_id: string;
        chat_id: string;
        archived: boolean;
      };
    }
  | { ScanTarget: { value: string } }
  | {
      StartProfileChat: {
        profile: HostedChatProfile;
        display_name: string;
      };
    }
  | {
      StartGroupChat: {
        profiles: HostedChatProfile[];
        display_name: string;
      };
    }
  | { SendMessage: { room_id: string; text: string } }
  | { SendTopicMessage: { room_id: string; topic_id: string; text: string } }
  | {
      SendChatMessage: {
        room_id: string;
        topic_id: string;
        chat_id: string;
        text: string;
      };
    }
  | {
      AnswerClarification: {
        room_id: string;
        topic_id: string;
        chat_id: string;
        request_id: string;
        text: string;
      };
    }
  | { LoadOlderMessages: { room_id: string; before_message_id: string; limit: number } }
  | { MarkRoomRead: { room_id: string } }
  | { SetTyping: { room_id: string; is_typing: boolean } }
  | { RefreshDevices: null }
  | { RevokeDevice: { account_id: string; device_id: string } };

export type HostedDeviceConfig = {
  baseUrl: string;
  apiToken: string;
};

export type BrainIdentityProviderRequest = {
  version: "finite-brain-identity-provider-v1";
  operation:
    | "identifyMember"
    | "authorizeHttpRequest"
    | "authorizeBrainEvent"
    | "openGrantPayload"
    | "wrapGrantPayload";
  input: unknown;
};

export type BrainIdentityProviderResponse = Record<string, unknown>;

export type SitesIdentityProviderRequest = {
  version: "finite-sites-identity-provider-v1";
  operation: "authorizeViewerSession";
  input: {
    url: string;
    returnTo: string;
    client: string;
    nonce: string;
  };
};

export type SitesIdentityProviderResponse = {
  body_json: string;
  authorization_header: string;
};

export type HostedRuntimeCommand = {
  room_id: string;
  conversation_id?: string | null;
  target_account_id: string;
  command: string;
  resource_key?: string | null;
  schema: string;
  body: unknown;
  reuse_succeeded_owner_claim?: boolean;
  wait_millis?: number;
};

export type HostedRuntimeCommandResponse = {
  request_id: string;
  status: "succeeded" | "failed" | "cancelled";
  body?: unknown;
  error?: { code: string; message: string } | null;
};

export type HostedDeviceLinkRequest = {
  pairing_session_id: string;
  target_device_id: string;
};

export type HostedDeviceEnrollmentRequest = HostedDeviceLinkRequest & {
  enrollment_user_id: string;
  enrollment_capability_hex: string;
};

export type HostedDeviceLinkStatus =
  | "awaiting_offer"
  | "awaiting_key_package"
  | "joining_rooms"
  | "ready"
  | "expired";

export type HostedDeviceLinkResponse = HostedDeviceLinkRequest & {
  status: HostedDeviceLinkStatus;
  expires_at_unix_seconds: number;
  room_count: number;
  active_room_count: number;
  bootstrap_manifests: {
    bootstrap_id: string;
    room_id: string;
    manifest_sha256: string;
  }[];
  source_descriptor?: {
    version: number;
    source_public_key: string;
    session_secret_hex: string;
    expires_at_unix_seconds: number;
  };
};

export type HostedDeviceReconcileRequest = {
  project_id: string;
  target_device_id: string;
};

export type HostedDeviceReconcileStatus =
  | "awaiting_key_package"
  | "joining_rooms"
  | "ready";

export type HostedDeviceReconcileResponse = HostedDeviceReconcileRequest & {
  status: HostedDeviceReconcileStatus;
  room_count: number;
  active_room_count: number;
};

export function hostedDeviceConfig(
  env: Record<string, string | undefined> = process.env
): HostedDeviceConfig | null {
  const baseUrl = env.FC_HOSTED_WEB_DEVICE_URL?.trim().replace(/\/+$/u, "");
  const apiToken = env.FINITECHAT_HOSTED_API_TOKEN?.trim();
  if (!baseUrl || !apiToken) {
    return null;
  }
  const parsed = new URL(baseUrl);
  if (parsed.protocol !== "http:" && parsed.protocol !== "https:") {
    throw new Error("FC_HOSTED_WEB_DEVICE_URL must use http or https");
  }
  return { baseUrl, apiToken };
}

export function hostedDeviceHeaders(
  config: HostedDeviceConfig,
  account: AccountAuthContext,
  contentType = false
) {
  if (!account.workosUserId || !account.emailVerified) {
    throw new Error("Sign in again to use chat.");
  }
  const headers = new Headers({
    accept: "application/json",
    authorization: `Bearer ${config.apiToken}`,
    "x-finite-workos-user-id": account.workosUserId,
  });
  if (contentType) {
    headers.set("content-type", "application/json");
  }
  return headers;
}

export async function hostedDeviceState(
  config: HostedDeviceConfig,
  account: AccountAuthContext
) {
  return hostedDeviceJson<HostedChatState>(config, account, "/v1/app/state");
}

export async function hostedDeviceBrainIdentityProvider(
  config: HostedDeviceConfig,
  account: AccountAuthContext,
  request: BrainIdentityProviderRequest,
  brainPublicOrigin: string
) {
  const parsedOrigin = new URL(brainPublicOrigin);
  if (parsedOrigin.origin !== brainPublicOrigin.replace(/\/$/u, "")) {
    throw new Error("Brain public origin must not include a path, query, or fragment.");
  }
  const headers = hostedDeviceHeaders(config, account, true);
  headers.set("x-finite-brain-public-origin", parsedOrigin.origin);
  const response = await fetch(`${config.baseUrl}/v1/brain/identity-provider`, {
    method: "POST",
    cache: "no-store",
    headers,
    body: JSON.stringify(request),
    signal: AbortSignal.timeout(HOSTED_DEVICE_TIMEOUT_MS),
  });
  if (!response.ok) {
    throw new HostedDeviceRequestError(await responseError(response), response.status);
  }
  return response.json() as Promise<BrainIdentityProviderResponse>;
}

export async function hostedDeviceSitesIdentityProvider(
  config: HostedDeviceConfig,
  account: AccountAuthContext,
  request: SitesIdentityProviderRequest,
  sitesPublicOrigin: string
) {
  const parsedOrigin = new URL(sitesPublicOrigin);
  if (parsedOrigin.origin !== sitesPublicOrigin.replace(/\/$/u, "")) {
    throw new Error("Sites public origin must not include a path, query, or fragment.");
  }
  const headers = hostedDeviceHeaders(config, account, true);
  headers.set("x-finite-sites-public-origin", parsedOrigin.origin);
  const response = await fetch(`${config.baseUrl}/v1/sites/identity-provider`, {
    method: "POST",
    cache: "no-store",
    headers,
    body: JSON.stringify(request),
    signal: AbortSignal.timeout(HOSTED_DEVICE_TIMEOUT_MS),
  });
  if (!response.ok) {
    throw new HostedDeviceRequestError(await responseError(response), response.status);
  }
  return response.json() as Promise<SitesIdentityProviderResponse>;
}

export async function hostedDeviceAction(
  config: HostedDeviceConfig,
  account: AccountAuthContext,
  action: HostedChatAction
) {
  return hostedDeviceJson<HostedChatState>(config, account, "/v1/app/actions", {
    method: "POST",
    body: JSON.stringify(action),
  });
}

export async function hostedDeviceOpenAgentBinding(
  config: HostedDeviceConfig,
  account: AccountAuthContext,
  projectId: string
) {
  return hostedDeviceJson<HostedChatState>(config, account, "/v1/app/agent-bindings/open", {
    method: "POST",
    body: JSON.stringify({ project_id: projectId }),
  });
}

export async function hostedDeviceEnsureAgentBinding(
  config: HostedDeviceConfig,
  account: AccountAuthContext,
  input: { project_id: string; agent_npub: string; display_name: string }
) {
  return hostedDeviceJson<HostedChatState>(config, account, "/v1/app/agent-bindings/ensure", {
    method: "POST",
    body: JSON.stringify(input),
  });
}

export async function hostedDeviceAuthorizeAgentBinding(
  config: HostedDeviceConfig,
  account: AccountAuthContext,
  input: { project_id: string; creation_request_id: string }
) {
  return hostedDeviceJson<HostedAgentBindingBootstrapAuthorization>(
    config,
    account,
    "/v1/app/agent-bindings/authorize-bootstrap",
    {
      method: "POST",
      body: JSON.stringify(input),
    }
  );
}

export async function hostedDeviceNewChat(
  config: HostedDeviceConfig,
  account: AccountAuthContext,
  input: Extract<HostedChatAction, { StartTopicChatIntent: unknown }>["StartTopicChatIntent"] & {
    project_id: string;
  }
) {
  return hostedDeviceJson<HostedChatState>(config, account, "/v1/app/new-chat", {
    method: "POST",
    body: JSON.stringify(input),
  });
}

export async function hostedDeviceRuntimeCommand(
  config: HostedDeviceConfig,
  account: AccountAuthContext,
  command: HostedRuntimeCommand
) {
  return hostedDeviceJson<HostedRuntimeCommandResponse>(
    config,
    account,
    "/v1/app/runtime-commands",
    {
      method: "POST",
      body: JSON.stringify(command),
    },
    65_000
  );
}

export async function hostedDeviceApproveLink(
  config: HostedDeviceConfig,
  account: AccountAuthContext,
  input: HostedDeviceLinkRequest
) {
  const result = await hostedDeviceJson<unknown>(
    config,
    account,
    "/v1/device-links/approve",
    {
      method: "POST",
      body: JSON.stringify(input),
    }
  );
  return parseHostedDeviceLinkResponse(result, input);
}

export async function hostedDeviceLinkStatus(
  config: HostedDeviceConfig,
  account: AccountAuthContext,
  input: HostedDeviceLinkRequest
) {
  const result = await hostedDeviceJson<unknown>(
    config,
    account,
    "/v1/device-links/status",
    {
      method: "POST",
      body: JSON.stringify(input),
    }
  );
  return parseHostedDeviceLinkResponse(result, input);
}

export async function hostedDeviceResumeEnrollment(
  config: HostedDeviceConfig,
  input: HostedDeviceEnrollmentRequest
) {
  const response = await fetch(`${config.baseUrl}/v1/device-links/enroll`, {
    method: "POST",
    cache: "no-store",
    headers: {
      accept: "application/json",
      authorization: `Bearer ${config.apiToken}`,
      "content-type": "application/json",
    },
    body: JSON.stringify(input),
    signal: AbortSignal.timeout(HOSTED_DEVICE_TIMEOUT_MS),
  });
  if (!response.ok) {
    throw new HostedDeviceRequestError(await responseError(response), response.status);
  }
  return parseHostedDeviceLinkResponse(await response.json(), input);
}

export async function hostedDeviceReconcileDevice(
  config: HostedDeviceConfig,
  account: AccountAuthContext,
  input: HostedDeviceReconcileRequest
) {
  const result = await hostedDeviceJson<unknown>(
    config,
    account,
    "/v1/device-links/reconcile",
    {
      method: "POST",
      body: JSON.stringify(input),
    }
  );
  return parseHostedDeviceReconcileResponse(result, input);
}

export async function hostedDeviceUpdates(
  config: HostedDeviceConfig,
  account: AccountAuthContext,
  signal: AbortSignal
) {
  return fetch(`${config.baseUrl}/v1/app/updates`, {
    cache: "no-store",
    headers: hostedDeviceHeaders(config, account),
    signal,
  });
}

export async function hostedDeviceAttachments(
  config: HostedDeviceConfig,
  account: AccountAuthContext,
  formData: FormData
) {
  return hostedDeviceJson<HostedChatState>(config, account, "/v1/app/attachments", {
    method: "POST",
    body: formData,
  });
}

export async function hostedDeviceProfileImage(
  config: HostedDeviceConfig,
  account: AccountAuthContext,
  bytes: Blob,
) {
  const contentType = bytes.type.trim().toLowerCase();
  if (!contentType.startsWith("image/")) {
    throw new Error("Choose an image file.");
  }
  const headers = hostedDeviceHeaders(config, account);
  headers.set("content-type", contentType);
  const response = await fetch(`${config.baseUrl}/v1/app/images`, {
    method: "POST",
    cache: "no-store",
    headers,
    body: bytes,
    signal: AbortSignal.timeout(HOSTED_DEVICE_TIMEOUT_MS),
  });
  if (!response.ok) {
    throw new HostedDeviceRequestError(await responseError(response), response.status);
  }
  const result = (await response.json()) as { image_url?: unknown };
  if (typeof result.image_url !== "string" || !result.image_url.trim()) {
    throw new Error("The image upload did not finish.");
  }
  return result.image_url;
}

export async function hostedDeviceAttachment(
  config: HostedDeviceConfig,
  account: AccountAuthContext,
  roomId: string,
  messageId: string,
  attachmentId: string,
  signal: AbortSignal
) {
  const path = [roomId, messageId, attachmentId]
    .map((value) => encodeURIComponent(value))
    .join("/");
  return fetch(`${config.baseUrl}/v1/app/attachments/${path}`, {
    cache: "no-store",
    headers: hostedDeviceHeaders(config, account),
    signal,
  });
}

async function hostedDeviceJson<T>(
  config: HostedDeviceConfig,
  account: AccountAuthContext,
  path: string,
  init: RequestInit = {},
  timeoutMs = HOSTED_DEVICE_TIMEOUT_MS
): Promise<T> {
  const diagnosticPath = hostedDeviceDiagnosticPath(path);
  let response: Response;
  try {
    response = await fetch(`${config.baseUrl}${path}`, {
      ...init,
      cache: "no-store",
      headers: hostedDeviceHeaders(config, account, typeof init.body === "string"),
      signal: AbortSignal.timeout(timeoutMs),
    });
  } catch (error) {
    console.error("hosted-device request failed", {
      path: diagnosticPath,
      errorClass:
        error instanceof DOMException && error.name === "TimeoutError"
          ? "timeout"
          : "transport",
    });
    throw error;
  }
  if (!response.ok) {
    console.error("hosted-device request failed", {
      path: diagnosticPath,
      status: response.status,
      errorClass: "downstream_http",
    });
    throw new HostedDeviceRequestError(await responseError(response), response.status);
  }
  return response.json() as Promise<T>;
}

export function hostedDeviceDiagnosticPath(path: string) {
  const pathname = path.split("?", 1)[0];
  if (pathname.startsWith("/v1/app/attachments/")) {
    return "/v1/app/attachments/:room/:message/:attachment";
  }
  if (pathname.startsWith("/v1/device-links/")) {
    return "/v1/device-links/:operation";
  }
  if (
    new Set([
      "/v1/app/actions",
      "/v1/app/agent-bindings/authorize-bootstrap",
      "/v1/app/agent-bindings/ensure",
      "/v1/app/agent-bindings/open",
      "/v1/app/attachments",
      "/v1/app/new-chat",
      "/v1/app/runtime-commands",
      "/v1/app/state",
    ]).has(pathname)
  ) {
    return pathname;
  }
  if (pathname.startsWith("/v1/brain/")) {
    return "/v1/brain/:operation";
  }
  if (pathname.startsWith("/v1/sites/")) {
    return "/v1/sites/:operation";
  }
  return "/unknown";
}

function parseHostedDeviceLinkResponse(
  value: unknown,
  expected: HostedDeviceLinkRequest
): HostedDeviceLinkResponse {
  if (!value || typeof value !== "object") {
    throw new Error("Device-link service returned an invalid response.");
  }
  const record = value as Record<string, unknown>;
  const statuses = new Set<HostedDeviceLinkStatus>([
    "awaiting_offer",
    "awaiting_key_package",
    "joining_rooms",
    "ready",
    "expired",
  ]);
  const status = record.status;
  const expiresAt = record.expires_at_unix_seconds;
  const roomCount = record.room_count;
  const activeRoomCount = record.active_room_count;
  const bootstrapManifests =
    record.bootstrap_manifests === undefined ? [] : record.bootstrap_manifests;
  const manifestKeys = Array.isArray(bootstrapManifests)
    ? bootstrapManifests.map((manifest) => {
        if (!manifest || typeof manifest !== "object" || Array.isArray(manifest)) {
          return "";
        }
        const item = manifest as Record<string, unknown>;
        return `${String(item.bootstrap_id)}\u0000${String(item.room_id)}`;
      })
    : [];
  const descriptor = parsePairingSourceDescriptor(record.source_descriptor);
  if (
    record.pairing_session_id !== expected.pairing_session_id ||
    record.target_device_id !== expected.target_device_id ||
    typeof status !== "string" ||
    !statuses.has(status as HostedDeviceLinkStatus) ||
    !Number.isSafeInteger(expiresAt) ||
    (expiresAt as number) < 0 ||
    !Number.isSafeInteger(roomCount) ||
    (roomCount as number) < 0 ||
    !Number.isSafeInteger(activeRoomCount) ||
    (activeRoomCount as number) < 0 ||
    (activeRoomCount as number) > (roomCount as number) ||
    !Array.isArray(bootstrapManifests) ||
    bootstrapManifests.some((manifest) => {
      if (!manifest || typeof manifest !== "object" || Array.isArray(manifest)) {
        return true;
      }
      const item = manifest as Record<string, unknown>;
      return (
        Object.keys(item).sort().join(",") !==
          "bootstrap_id,manifest_sha256,room_id" ||
        typeof item.bootstrap_id !== "string" ||
        item.bootstrap_id.length === 0 ||
        typeof item.room_id !== "string" ||
        item.room_id.length === 0 ||
        typeof item.manifest_sha256 !== "string" ||
        !/^[0-9a-f]{64}$/u.test(item.manifest_sha256)
      );
    }) ||
    new Set(manifestKeys).size !== manifestKeys.length ||
    (status === "ready" && bootstrapManifests.length !== roomCount)
  ) {
    throw new Error("Device-link service returned an invalid response.");
  }
  // Project an exact allowlist. Even an accidentally expanded internal
  // response can never forward encrypted or signer material to the browser.
  return {
    pairing_session_id: expected.pairing_session_id,
    target_device_id: expected.target_device_id,
    status: status as HostedDeviceLinkStatus,
    expires_at_unix_seconds: expiresAt as number,
    room_count: roomCount as number,
    active_room_count: activeRoomCount as number,
    bootstrap_manifests: bootstrapManifests as HostedDeviceLinkResponse["bootstrap_manifests"],
    ...(descriptor ? { source_descriptor: descriptor } : {}),
  };
}

function parsePairingSourceDescriptor(value: unknown) {
  if (value === undefined) return undefined;
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new Error("Device-link service returned an invalid source descriptor.");
  }
  const record = value as Record<string, unknown>;
  const keys = Object.keys(record).sort();
  if (
    keys.join(",") !==
      "expires_at_unix_seconds,session_secret_hex,source_public_key,version" ||
    record.version !== 1 ||
    typeof record.source_public_key !== "string" ||
    !/^[0-9a-f]{64}$/u.test(record.source_public_key) ||
    typeof record.session_secret_hex !== "string" ||
    !/^[0-9a-f]{64}$/u.test(record.session_secret_hex) ||
    !Number.isSafeInteger(record.expires_at_unix_seconds) ||
    (record.expires_at_unix_seconds as number) < 0
  ) {
    throw new Error("Device-link service returned an invalid source descriptor.");
  }
  return {
    version: 1,
    source_public_key: record.source_public_key,
    session_secret_hex: record.session_secret_hex,
    expires_at_unix_seconds: record.expires_at_unix_seconds as number,
  };
}

function parseHostedDeviceReconcileResponse(
  value: unknown,
  expected: HostedDeviceReconcileRequest
): HostedDeviceReconcileResponse {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new Error("Device reconciliation service returned an invalid response.");
  }
  const record = value as Record<string, unknown>;
  const statuses = new Set<HostedDeviceReconcileStatus>([
    "awaiting_key_package",
    "joining_rooms",
    "ready",
  ]);
  const status = record.status;
  const roomCount = record.room_count;
  const activeRoomCount = record.active_room_count;
  if (
    record.project_id !== expected.project_id ||
    record.target_device_id !== expected.target_device_id ||
    typeof status !== "string" ||
    !statuses.has(status as HostedDeviceReconcileStatus) ||
    !Number.isSafeInteger(roomCount) ||
    (roomCount as number) < 0 ||
    !Number.isSafeInteger(activeRoomCount) ||
    (activeRoomCount as number) < 0 ||
    (activeRoomCount as number) > (roomCount as number)
  ) {
    throw new Error("Device reconciliation service returned an invalid response.");
  }
  // Project an exact allowlist so internal signer or recovery material can
  // never cross the dashboard boundary if the service response expands.
  return {
    project_id: expected.project_id,
    target_device_id: expected.target_device_id,
    status: status as HostedDeviceReconcileStatus,
    room_count: roomCount as number,
    active_room_count: activeRoomCount as number,
  };
}

async function responseError(response: Response) {
  const text = await response.text();
  try {
    const parsed = JSON.parse(text) as { error?: unknown };
    if (typeof parsed.error === "string" && parsed.error.trim()) {
      return parsed.error;
    }
  } catch {
    // Preserve the bounded plain-text response below.
  }
  return text.slice(0, 500) || "Chat is unavailable right now.";
}
