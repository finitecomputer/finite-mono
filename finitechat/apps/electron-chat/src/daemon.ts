import type { AppAction, AppState } from "@finite/chat-ui";

export type { AppAction, AppState } from "@finite/chat-ui";

export type AttachmentUpload = {
  room_id: string;
  topic_id?: string | null;
  chat_id?: string | null;
  caption: string;
  reply_to_message_id?: string | null;
  files: {
    filename: string;
    mime_type: string;
    bytes: ArrayBuffer;
  }[];
};

const DESKTOP_IPC_CONNECTION = "finitechat-desktop-ipc";
const ATTACHMENT_MEDIA_ORIGIN = "finitechat-media://attachment";

export function attachmentMediaUrl(roomId: string, messageId: string, attachmentId: string) {
  return `${ATTACHMENT_MEDIA_ORIGIN}/${encodeURIComponent(roomId)}/${encodeURIComponent(messageId)}/${encodeURIComponent(attachmentId)}`;
}

export async function resolveDaemonUrl() {
  if (window.finiteChatDesktop) {
    return window.finiteChatDesktop.daemonConnection();
  }
  throw new Error("Finite Chat desktop bridge is unavailable");
}

export function getState(baseUrl: string) {
  if (baseUrl === DESKTOP_IPC_CONNECTION && window.finiteChatDesktop) {
    return window.finiteChatDesktop.daemonState();
  }
  return Promise.reject(new Error("Finite Chat desktop bridge is unavailable"));
}

export function dispatch(baseUrl: string, action: AppAction) {
  if (baseUrl === DESKTOP_IPC_CONNECTION && window.finiteChatDesktop) {
    return window.finiteChatDesktop.dispatchDaemonAction(action);
  }
  return Promise.reject(new Error("Finite Chat desktop bridge is unavailable"));
}

export function uploadAttachments(baseUrl: string, upload: AttachmentUpload) {
  if (baseUrl === DESKTOP_IPC_CONNECTION && window.finiteChatDesktop) {
    return window.finiteChatDesktop.uploadDaemonAttachments(upload);
  }
  return Promise.reject(new Error("Finite Chat desktop bridge is unavailable"));
}

export function subscribeToUpdates(
  baseUrl: string,
  onState: (state: AppState) => void,
  onError: (error: Error) => void,
  onGeneration?: (generation: { generation: number }) => void
) {
  if (baseUrl !== DESKTOP_IPC_CONNECTION || !window.finiteChatDesktop) {
    onError(new Error("Finite Chat desktop bridge is unavailable"));
    return () => {};
  }
  const unsubscribeState = window.finiteChatDesktop.onDaemonUpdate(onState);
  const unsubscribeError = window.finiteChatDesktop.onDaemonError((message) =>
    onError(new Error(message))
  );
  // Register last: this hook requests main's buffered replay, which always
  // emits generation before the first state from that daemon process.
  const unsubscribeGeneration = window.finiteChatDesktop.onDaemonGeneration((generation) =>
    onGeneration?.(generation)
  );
  return () => {
    unsubscribeState();
    unsubscribeError();
    unsubscribeGeneration();
  };
}
