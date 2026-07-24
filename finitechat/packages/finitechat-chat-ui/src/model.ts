/**
 * JSON presentation model emitted by finitechat-core.
 *
 * Keep this module dependency-free so Hosted Web and local Device renderers
 * consume one model without sharing either transport or secret custody.
 */

export type Identity = {
  account_id: string;
  device_id: string;
  /** Redacted to an empty string at HTTP boundaries. */
  account_secret_hex?: string;
};

export type AppRoomState =
  | "Connected"
  | "WaitingForApproval"
  | "Joining"
  | "UnavailableOnDevice";

export type AppRoomSummary = {
  room_id: string;
  display_name: string;
  picture?: string | null;
  state: AppRoomState;
  status: string;
  user_status_text: string;
  last_message_preview: string;
  unread_count: number;
  can_load_older: boolean;
  is_agent_chat: boolean;
};

export type AppChatSummary = {
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
  chats: AppChatSummary[];
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

export type AppRoomDetailsState = {
  room_id: string;
  display_name: string;
  picture?: string | null;
  state: AppRoomState;
  status: string;
  user_status_text: string;
  media_item_count: number;
  members: AppRoomMemberSummary[];
  devices: AppDeviceSummary[];
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

export type ChatMediaGalleryItem = {
  item_id: string;
  room_id: string;
  message_id: string;
  attachment_id: string;
  attachment: ChatMediaAttachment;
  sender_display_name: string;
  sender_npub?: string | null;
  timestamp_unix_seconds: number;
  display_timestamp: string;
};

export type ChatMediaGalleryState = {
  room_id: string;
  items: ChatMediaGalleryItem[];
};

export type ChatReactionSummary = {
  emoji: string;
  count: number;
  reacted_by_me: boolean;
};

export type ChatReadReceiptSummary = {
  delivered_count: number;
  read_count: number;
  display_text: string;
};

export type ChatPollOption = {
  option_id: string;
  text: string;
  vote_count: number;
  voted_by_me: boolean;
};

export type ChatPoll = {
  question: string;
  options: ChatPollOption[];
  total_votes: number;
  my_vote_option_id?: string | null;
};

export type OutboundDelivery = {
  local_send: "Sending" | "Sent";
  server_delivery: "Undelivered" | "Delivered" | { Failed: { reason: string } };
};

export type ChatMessageKind = "message" | "status" | "tool" | "media" | string;
export type ChatMessageStatus = "running" | "complete" | string;

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
  rich_text_json?: string;
  kind: ChatMessageKind;
  status: ChatMessageStatus;
  final_delivery: boolean;
  edit_of_message_id?: string | null;
  /** Not needed by renderers; present in the unredacted Rust record. */
  payload?: number[];
  reply_to_message_id?: string | null;
  is_mine: boolean;
  outbound_delivery?: OutboundDelivery | null;
  reactions?: ChatReactionSummary[];
  media: ChatMediaAttachment[];
  read_receipt?: ChatReadReceiptSummary | null;
  poll?: ChatPoll | null;
  timestamp_unix_seconds: number;
  display_timestamp: string;
};

export type AppTypingMember = {
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

export type AppFlowState = {
  notice_text?: string | null;
  notice_busy: boolean;
  scan_in_flight: boolean;
  scan_result: "none" | "profile" | "unavailable" | string;
  image_upload_url?: string | null;
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
  media_gallery?: ChatMediaGalleryState | null;
  room_details?: AppRoomDetailsState | null;
  profiles: AppProfileSummary[];
  devices: AppDeviceSummary[];
  typing_members: AppTypingMember[];
  flow: AppFlowState;
};

export type OutboundAttachment = {
  filename: string;
  mime_type: string;
  kind: ChatMediaKind;
  bytes: number[];
};

/** Externally tagged serde representation used by the HTTP renderers. */
export type AppAction =
  | { StartRuntime: null }
  | { StopRuntime: null }
  | { OpenRoom: { room_id: string } }
  | { OpenTopic: { room_id: string; topic_id: string } }
  | { OpenChat: { room_id: string; topic_id: string; chat_id: string } }
  | { RenameChat: { room_id: string; topic_id: string; chat_id: string; title: string } }
  | { SetChatArchived: { room_id: string; topic_id: string; chat_id: string; archived: boolean } }
  | { CreateRoom: { display_name: string } }
  | { CreateTopic: { room_id: string; title: string } }
  | { StartTopicChat: { room_id: string; topic_id: string; reason?: string | null } }
  | { SaveProfile: { display_name: string; about: string; picture?: string | null } }
  | { UploadImage: { bytes: number[]; content_type: string } }
  | { SaveRoomMetadata: { room_id: string; display_name: string; picture?: string | null } }
  | { StartProfileChat: { profile: AppProfileSummary; display_name: string } }
  | { StartGroupChat: { profiles: AppProfileSummary[]; display_name: string } }
  | { AddRoomMembers: { room_id: string; profiles: AppProfileSummary[] } }
  | { ScanTarget: { value: string } }
  | { SendMessage: { room_id: string; text: string } }
  | { SendTopicMessage: { room_id: string; topic_id: string; text: string } }
  | { SendChatMessage: { room_id: string; topic_id: string; chat_id: string; text: string } }
  | { SendReply: { room_id: string; text: string; reply_to_message_id: string } }
  | {
      SendChatReply: {
        room_id: string;
        topic_id: string;
        chat_id: string;
        text: string;
        reply_to_message_id: string;
      };
    }
  | {
      SendAttachment: {
        room_id: string;
        filename: string;
        mime_type: string;
        kind: ChatMediaKind;
        bytes: number[];
        caption: string;
        reply_to_message_id?: string | null;
      };
    }
  | {
      SendAttachments: {
        room_id: string;
        attachments: OutboundAttachment[];
        caption: string;
        reply_to_message_id?: string | null;
      };
    }
  | {
      SendChatAttachment: {
        room_id: string;
        topic_id: string;
        chat_id: string;
        filename: string;
        mime_type: string;
        kind: ChatMediaKind;
        bytes: number[];
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
  | { VotePoll: { room_id: string; message_id: string; option_id: string } }
  | { DownloadAttachment: { room_id: string; message_id: string; attachment_id: string } }
  | { BeginDownloadAttachment: { room_id: string; message_id: string; attachment_id: string } }
  | { LoadOlderMessages: { room_id: string; before_message_id: string; limit: number } }
  | { ReactToMessage: { room_id: string; message_id: string; emoji: string } }
  | { MarkRoomRead: { room_id: string } }
  | { RetryMessage: { room_id: string; message_id: string } }
  | { SetTyping: { room_id: string; is_typing: boolean } }
  | { RefreshDevices: null }
  | { RevokeDevice: { account_id: string; device_id: string } }
  | { SetPushToken: { token: string } }
  | { RemovePushToken: null };
