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
  flow: {
    notice_text?: string | null;
    notice_busy: boolean;
    scan_in_flight: boolean;
    scan_result: string;
    image_upload_url?: string | null;
  };
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
      RenameChat: {
        room_id: string;
        topic_id: string;
        chat_id: string;
        title: string;
      };
    }
  | { ScanTarget: { value: string } }
  | {
      StartProfileChat: {
        profile: HostedChatProfile;
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
  | { LoadOlderMessages: { room_id: string; before_message_id: string; limit: number } }
  | { MarkRoomRead: { room_id: string } }
  | { SetTyping: { room_id: string; is_typing: boolean } }
  | { RefreshDevices: null }
  | { RevokeDevice: { account_id: string; device_id: string } };

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
