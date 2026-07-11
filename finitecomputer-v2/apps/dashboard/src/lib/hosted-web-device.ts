import type { AccountAuthContext } from "@/lib/dashboard-auth";
import type {
  AppAction,
  AppChatSummary,
  AppDeviceSummary,
  AppProfileSummary,
  AppRoomSummary,
  AppState,
  AppTopicSummary,
  AppTypingMember,
  ChatMediaAttachment,
  ChatMediaKind,
  ChatMessage,
  OutboundDelivery,
} from "@finite/chat-ui";

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

// The browser and desktop consume the exact same finitechat-core contract.
// Keep the Hosted names for the server modules that predate the shared package,
// but never maintain a second, narrower copy of the wire model here.
export type HostedChatRoom = AppRoomSummary;
export type HostedChatSummary = AppChatSummary;
export type HostedChatTopic = AppTopicSummary;
export type HostedChatMediaKind = ChatMediaKind;
export type HostedChatMediaAttachment = ChatMediaAttachment;
export type HostedChatOutboundDelivery = OutboundDelivery;
export type HostedChatMessage = ChatMessage;
export type HostedChatTypingMember = AppTypingMember;
export type HostedChatProfile = AppProfileSummary;
export type HostedChatDevice = AppDeviceSummary;
export type HostedChatState = AppState;
export type HostedChatAction = AppAction;

export type HostedDeviceConfig = {
  baseUrl: string;
  apiToken: string;
};

export type HostedRuntimeCommand = {
  room_id: string;
  conversation_id?: string | null;
  target_account_id: string;
  command: string;
  resource_key?: string | null;
  schema: string;
  body: unknown;
  wait_millis?: number;
};

export type HostedRuntimeCommandResponse = {
  request_id: string;
  status: "succeeded" | "failed" | "cancelled";
  body?: unknown;
  error?: { code: string; message: string } | null;
};

export type HostedDeviceLinkRequest = {
  link_session_id: string;
  target_device_id: string;
};

export type HostedDeviceLinkStatus =
  | "awaiting_claim"
  | "awaiting_key_package"
  | "joining_rooms"
  | "ready"
  | "expired";

export type HostedDeviceLinkResponse = HostedDeviceLinkRequest & {
  status: HostedDeviceLinkStatus;
  expires_at_unix_seconds: number;
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
  const response = await fetch(`${config.baseUrl}${path}`, {
    ...init,
    cache: "no-store",
    headers: hostedDeviceHeaders(config, account, typeof init.body === "string"),
    signal: AbortSignal.timeout(timeoutMs),
  });
  if (!response.ok) {
    throw new HostedDeviceRequestError(await responseError(response), response.status);
  }
  return response.json() as Promise<T>;
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
    "awaiting_claim",
    "awaiting_key_package",
    "joining_rooms",
    "ready",
    "expired",
  ]);
  const status = record.status;
  const expiresAt = record.expires_at_unix_seconds;
  const roomCount = record.room_count;
  const activeRoomCount = record.active_room_count;
  if (
    record.link_session_id !== expected.link_session_id ||
    record.target_device_id !== expected.target_device_id ||
    typeof status !== "string" ||
    !statuses.has(status as HostedDeviceLinkStatus) ||
    !Number.isSafeInteger(expiresAt) ||
    (expiresAt as number) < 0 ||
    !Number.isSafeInteger(roomCount) ||
    (roomCount as number) < 0 ||
    !Number.isSafeInteger(activeRoomCount) ||
    (activeRoomCount as number) < 0 ||
    (activeRoomCount as number) > (roomCount as number)
  ) {
    throw new Error("Device-link service returned an invalid response.");
  }
  // Project an exact allowlist. Even an accidentally expanded internal
  // response can never forward encrypted or signer material to the browser.
  return {
    link_session_id: expected.link_session_id,
    target_device_id: expected.target_device_id,
    status: status as HostedDeviceLinkStatus,
    expires_at_unix_seconds: expiresAt as number,
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
