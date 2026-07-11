import type { AppAction, AppState } from "../model";

export type ChatProductUpload = {
  room_id: string;
  topic_id?: string | null;
  chat_id?: string | null;
  caption: string;
  reply_to_message_id?: string | null;
  files: File[];
};

export type ChatAttachmentCoordinates = {
  room_id: string;
  message_id: string;
  attachment_id: string;
};

/**
 * The sole host boundary for the shared chat product. Web implements this
 * with authenticated HTTP/EventSource and Desktop implements it with IPC to
 * the local daemon. Neither transport leaks into the product DOM.
 */
export type ChatTransport = {
  load(): Promise<AppState>;
  dispatch(action: AppAction): Promise<AppState>;
  upload?(upload: ChatProductUpload): Promise<AppState>;
  subscribe?(
    onState: (state: AppState) => void,
    onError: (error: Error) => void,
    onReset: () => void
  ): () => void;
  attachmentUrl?(attachment: ChatAttachmentCoordinates): string;
};

export type ChatProductLink = {
  href?: string;
  onClick?: () => void | Promise<void>;
  label: string;
};

export type ChatProductPreview = {
  /** Exchange a published site URL for an authorized, embeddable URL. */
  createSession(url: string): Promise<string>;
};
