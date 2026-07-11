import type {
  ChatAttachmentCoordinates,
  ChatProductPreview,
  ChatProductUpload,
  ChatTransport,
} from "@finite/chat-ui/react";
import type { AppAction, AppState } from "@finite/chat-ui";

const STREAM_RECONNECT_DELAY_MS = 1_000;

export type HostedWebChatProductTransport = ChatTransport & {
  close(): void;
};

export function createHostedWebChatTransport(
  machineId: string
): HostedWebChatProductTransport {
  const apiBase = `/api/chat/machines/${encodeURIComponent(machineId)}/hosted-device`;
  let closeCurrentSubscription: (() => void) | null = null;

  return {
    load: () => chatRequest<AppState>(`${apiBase}/state`),
    dispatch: (action: AppAction) =>
      chatRequest<AppState>(`${apiBase}/actions`, {
        method: "POST",
        body: JSON.stringify(action),
      }),
    async upload(upload: ChatProductUpload) {
      const formData = new FormData();
      formData.set("room_id", upload.room_id);
      if (upload.topic_id) formData.set("topic_id", upload.topic_id);
      if (upload.chat_id) formData.set("chat_id", upload.chat_id);
      if (upload.reply_to_message_id) {
        formData.set("reply_to_message_id", upload.reply_to_message_id);
      }
      formData.set("caption", upload.caption);
      for (const file of upload.files) formData.append("files", file);
      return chatRequest<AppState>(`${apiBase}/attachments`, {
        method: "POST",
        body: formData,
      });
    },
    subscribe(onState, onError, onReset) {
      closeCurrentSubscription?.();
      let disposed = false;
      let events: EventSource | null = null;
      let reconnectTimer: ReturnType<typeof setTimeout> | null = null;

      const connect = () => {
        if (disposed) return;
        onReset();
        const nextEvents = new EventSource(`${apiBase}/updates`);
        events = nextEvents;
        nextEvents.addEventListener("state", ((event: MessageEvent<string>) => {
          if (disposed || events !== nextEvents) return;
          try {
            onState(JSON.parse(event.data) as AppState);
          } catch {
            onError(new Error("Chat received an update it could not read."));
          }
        }) as EventListener);
        nextEvents.addEventListener("error", () => {
          if (disposed || events !== nextEvents) return;
          nextEvents.close();
          events = null;
          onError(new Error("Reconnecting…"));
          reconnectTimer = setTimeout(connect, STREAM_RECONNECT_DELAY_MS);
        });
      };

      connect();
      const close = () => {
        disposed = true;
        if (reconnectTimer) clearTimeout(reconnectTimer);
        events?.close();
        if (closeCurrentSubscription === close) closeCurrentSubscription = null;
      };
      closeCurrentSubscription = close;
      return close;
    },
    attachmentUrl(coordinates: ChatAttachmentCoordinates) {
      return `${apiBase}/attachments/${encodeURIComponent(coordinates.room_id)}/${encodeURIComponent(coordinates.message_id)}/${encodeURIComponent(coordinates.attachment_id)}`;
    },
    close() {
      closeCurrentSubscription?.();
    },
  };
}

export function createHostedSitePreview(machineId: string): ChatProductPreview {
  return {
    async createSession(url: string) {
      const response = await fetch(
        `/api/site-previews/machines/${encodeURIComponent(machineId)}/session`,
        {
          method: "POST",
          headers: { "content-type": "application/json" },
          body: JSON.stringify({ url }),
        }
      );
      if (!response.ok) throw new Error("preview session unavailable");
      const payload = (await response.json()) as { url?: unknown };
      if (typeof payload.url !== "string" || !payload.url) {
        throw new Error("preview session unavailable");
      }
      return payload.url;
    },
  };
}

async function chatRequest<T>(url: string, init: RequestInit = {}): Promise<T> {
  const response = await fetch(url, {
    ...init,
    cache: "no-store",
    headers: {
      ...(init.body instanceof FormData ? {} : { "content-type": "application/json" }),
      ...init.headers,
    },
  });
  if (!response.ok) {
    const payload = (await response.json().catch(() => null)) as
      | { error?: unknown }
      | null;
    throw new Error(
      typeof payload?.error === "string" && payload.error.trim()
        ? payload.error
        : `Chat request failed (${response.status}).`
    );
  }
  return (await response.json()) as T;
}
