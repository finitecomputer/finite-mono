export type Identity = {
  account_id: string;
  device_id: string;
  account_secret_hex?: string;
};

export type AppRoomMemberSummary = {
  account_id: string;
  device_id: string;
  npub: string;
  display_name: string;
  picture?: string | null;
  current_device: boolean;
};

export type AppDeviceSummary = {
  account_id: string;
  device_id: string;
  active: boolean;
  current_device: boolean;
  revoked: boolean;
  room_count: number;
};

export type AppRoomSummary = {
  room_id: string;
  display_name: string;
  picture?: string | null;
  state: "Connected" | "WaitingForApproval" | "Joining" | "UnavailableOnDevice";
  status: string;
  user_status_text: string;
  last_message_preview: string;
  unread_count: number;
  can_load_older: boolean;
  is_agent_chat: boolean;
  media_item_count?: number;
  members?: AppRoomMemberSummary[];
  devices?: AppDeviceSummary[];
};

export type AppTopicSummary = {
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
  chats: {
    chat_id: string;
    title: string;
    last_message_preview: string;
    unread_count: number;
    message_count: number;
    started_seq: number;
    updated_seq: number;
    active: boolean;
  }[];
};

export type ChatMediaKind = "Image" | "VoiceNote" | "Video" | "File";

export type ChatMediaAttachment = {
  attachment_id: string;
  url?: string | null;
  mime_type: string;
  filename: string;
  kind: ChatMediaKind;
  width?: number | null;
  height?: number | null;
  local_path?: string | null;
  upload_progress_per_mille?: number | null;
  download_progress_per_mille?: number | null;
};

export type ChatReadReceiptSummary = {
  delivered_count: number;
  read_count: number;
  display_text: string;
};

export type OutboundDelivery = {
  local_send: "Sending" | "Sent";
  server_delivery: "Undelivered" | "Delivered" | { Failed: { reason: string } };
};

export type ChatMessage = {
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
  rich_text_json: string;
  reply_to_message_id?: string | null;
  is_mine: boolean;
  outbound_delivery?: OutboundDelivery | null;
  media: ChatMediaAttachment[];
  read_receipt?: ChatReadReceiptSummary | null;
  timestamp_unix_seconds: number;
  display_timestamp: string;
};

export type AppTypingMember = {
  room_id: string;
  account_id: string;
  device_id: string;
  display_name: string;
  picture?: string | null;
  npub?: string | null;
  activity_kind: "typing" | "thinking" | "working" | string;
};

export type AppProfileSummary = {
  account_id: string;
  npub: string;
  display_name: string;
  about?: string | null;
  picture?: string | null;
  stale: boolean;
  is_agent: boolean;
};

export type OutboundAttachment = {
  filename: string;
  mime_type: string;
  kind: ChatMediaKind;
  bytes: number[];
};

export type AppState = {
  rev: number;
  identity: Identity;
  rooms: AppRoomSummary[];
  selected_room_id?: string | null;
  topics: AppTopicSummary[];
  selected_topic_id?: string | null;
  selected_chat_id?: string | null;
  active_profile_id?: string | null;
  status: string;
  toast?: string | null;
  messages: ChatMessage[];
  profiles: AppProfileSummary[];
  devices: unknown[];
  typing_members: AppTypingMember[];
  flow: {
    notice_text?: string | null;
    notice_busy: boolean;
    scan_in_flight: boolean;
    scan_result: string;
    image_upload_url?: string | null;
  };
};

export type AppAction =
  | { StartRuntime: null }
  | { StopRuntime: null }
  | { OpenRoom: { room_id: string } }
  | { OpenTopic: { room_id: string; topic_id: string } }
  | { OpenChat: { room_id: string; topic_id: string; chat_id: string } }
  | { CreateRoom: { display_name: string } }
  | { CreateTopic: { room_id: string; title: string } }
  | { StartTopicChat: { room_id: string; topic_id: string; reason?: string | null } }
  | { StartProfileChat: { profile: AppProfileSummary; display_name: string } }
  | { StartGroupChat: { profiles: AppProfileSummary[]; display_name: string } }
  | { AddRoomMembers: { room_id: string; profiles: AppProfileSummary[] } }
  | { ScanTarget: { value: string } }
  | { SendMessage: { room_id: string; text: string } }
  | { SendTopicMessage: { room_id: string; topic_id: string; text: string } }
  | { SendChatMessage: { room_id: string; topic_id: string; chat_id: string; text: string } }
  | { SendReply: { room_id: string; text: string; reply_to_message_id: string } }
  | { SendChatReply: { room_id: string; topic_id: string; chat_id: string; text: string; reply_to_message_id: string } }
  | {
      SendAttachments: {
        room_id: string;
        attachments: OutboundAttachment[];
        caption: string;
        reply_to_message_id?: string | null;
      };
    }
  | {
      SendChatAttachments: {
        room_id: string;
        topic_id: string;
        chat_id: string;
        attachments: OutboundAttachment[];
        caption: string;
        reply_to_message_id?: string | null;
      };
    }
  | { SendPoll: { room_id: string; question: string; options: string[] } }
  | { SendChatPoll: { room_id: string; topic_id: string; chat_id: string; question: string; options: string[] } }
  | { LoadOlderMessages: { room_id: string; before_message_id: string; limit: number } }
  | { MarkRoomRead: { room_id: string } }
  | { RetryMessage: { room_id: string; message_id: string } }
  | { DownloadAttachment: { room_id: string; message_id: string; attachment_id: string } }
  | { SetTyping: { room_id: string; is_typing: boolean } };

export async function resolveDaemonUrl() {
  if (window.finiteChatDesktop) {
    return window.finiteChatDesktop.daemonUrl();
  }
  return import.meta.env.VITE_FINITECHAT_DAEMON_URL ?? "http://127.0.0.1:38917";
}

export async function daemonRequest<T>(baseUrl: string, path: string, init?: RequestInit): Promise<T> {
  const headers = new Headers(init?.headers);
  if (init?.body && !headers.has("content-type")) {
    headers.set("content-type", "application/json");
  }
  const response = await fetch(`${baseUrl}${path}`, { ...init, headers });
  if (!response.ok) {
    const text = await response.text();
    let message = text || `Request failed with ${response.status}`;
    try {
      const parsed = JSON.parse(text) as { error?: string };
      message = parsed.error || message;
    } catch {
      // Keep the raw text from the daemon.
    }
    throw new Error(message);
  }
  return response.json() as Promise<T>;
}

export function getState(baseUrl: string) {
  return daemonRequest<AppState>(baseUrl, "/v1/app/state");
}

export function dispatch(baseUrl: string, action: AppAction) {
  return daemonRequest<AppState>(baseUrl, "/v1/app/actions", {
    method: "POST",
    body: JSON.stringify(action),
  });
}

export function subscribeToUpdates(baseUrl: string, onState: (state: AppState) => void, onError: (error: Error) => void) {
  const events = new EventSource(`${baseUrl}/v1/app/updates`);
  events.addEventListener("state", (event) => {
    onState(JSON.parse((event as MessageEvent).data) as AppState);
  });
  events.addEventListener("error", () => {
    onError(new Error("Finite Chat daemon update stream disconnected"));
  });
  return () => events.close();
}
