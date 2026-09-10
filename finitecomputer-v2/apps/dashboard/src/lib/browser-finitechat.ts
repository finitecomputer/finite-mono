// Feasibility adapter: real dashboard projections over the real Rust/WASM Device.
// This module runs only in the browser. Nothing forwards plaintext to Next or Hermes HTTP.
import type { HostedChatAction, HostedChatMessage, HostedChatState, HostedChatTopic } from "./hosted-web-device";

type Payload = {
  type?: string; text?: string; kind?: string; status?: string; edit_of?: string;
  conversation_id?: string; segment_id?: string; topic_id?: string; chat_id?: string;
  title?: string; archived?: boolean; sender_name?: string; reply_to_message_id?: string;
  metadata?: Record<string, unknown>;
};
type Event = {
  id: string; seq: number; timestamp: number;
  sender: { account_id: string; device_id: string };
  kind: { type: string; name?: string };
  conversation_id: string | null; segment_id: string | null; payload: Payload | null;
};
type Snapshot = {
  room: string; device: { account_id: string; device_id: string };
  afterSeq: number; epoch: number; events: Event[];
};
type WasmChat = {
  connect: (agent: string, room: string) => Promise<void>;
  send_chat: (text: string, topic?: string, chat?: string, metadata?: string) => Promise<string>;
  publish_event: (kind: string, topic: string, payload: string) => Promise<string>;
  sync: () => Promise<string>;
  snapshot: () => string;
};
type WasmModule = {
  default: (options: { module_or_path: string }) => Promise<void>;
  BrowserChat: new (nsec: string, server: string, device: string) => WasmChat;
};
const nonNotifying = { push: "never", unread: "never", command_inbox: "never" };

export class BrowserFiniteChat {
  private client!: WasmChat;
  private agent!: string;
  private agentNpub!: string;
  private snapshot!: Snapshot;
  private topic = "home";
  private chat = "";
  private revision = 0;
  private queue: Promise<unknown> = Promise.resolve();
  private changed = new Set<() => void>();
  state: HostedChatState | null = null;
  error: string | null = null;
  connected = false;

  subscribe(listener: () => void) { this.changed.add(listener); return () => { this.changed.delete(listener); }; }
  private notify() { for (const listener of this.changed) listener(); }
  private serial<T>(operation: () => Promise<T>): Promise<T> {
    const result = this.queue.then(operation);
    this.queue = result.catch(() => {});
    return result;
  }
  private accept(raw: string) {
    this.snapshot = JSON.parse(raw) as Snapshot;
    this.connected = true;
    this.error = null;
    this.project();
    return this.state!;
  }
  async connect() {
    try {
      const moduleUrl = "/wasm-spike/finitechat_wasm.js";
      const wasm = await import(/* webpackIgnore: true */ /* turbopackIgnore: true */ moduleUrl) as WasmModule;
      await wasm.default({ module_or_path: "/wasm-spike/finitechat_wasm_bg.wasm" });
      const response = await fetch("/api/wasm-spike/bootstrap", { method: "POST", cache: "no-store" });
      if (!response.ok) throw new Error(`Local Core key handoff failed: ${response.status}`);
      const bootstrap = await response.json();
      this.agent = bootstrap.agentAccountId;
      this.agentNpub = bootstrap.agentNpub;
      const id = crypto.randomUUID();
      this.client = new wasm.BrowserChat(bootstrap.nsec, bootstrap.serverUrl, `browser_${id}`);
      bootstrap.nsec = "";
      await this.client.connect(this.agent, `wasm_${id}`);
      await this.publish("conversation_create", "home", { title: "Home", description: null, external_topic: null, skill_binding: null });
      await this.newChat("home");
      this.accept(this.client.snapshot());
    } catch (error) {
      this.error = String(error); this.connected = false; this.notify(); throw error;
    }
  }
  private async publish(type: string, topic: string, payload: object, name?: string) {
    const kind = name ? { type: "namespaced", name, policy: nonNotifying } : { type };
    this.snapshot = JSON.parse(await this.client.publish_event(JSON.stringify(kind), topic, JSON.stringify(payload)));
  }
  private async newChat(topic: string, intent?: string) {
    // A given intent maps to the same chat for this browser session.
    const id = `segment-${intent || crypto.randomUUID()}`;
    if (!this.snapshot.events.some(event => event.payload?.segment_id === id)) {
      await this.publish("conversation_segment_start", topic, { segment_id: id, reason: "new_chat" });
    }
    this.topic = topic; this.chat = id;
  }
  async sync() {
    return this.serial(async () => {
      try { return this.accept(await this.client.sync()); }
      catch (error) { this.error = String(error); this.connected = false; this.notify(); throw error; }
    });
  }
  async dispatch(action: HostedChatAction): Promise<HostedChatState> {
    return this.serial(async () => {
      if ("SendChatMessage" in action || "SendTopicMessage" in action || "SendMessage" in action) {
        const send = "SendChatMessage" in action ? action.SendChatMessage : "SendTopicMessage" in action ? action.SendTopicMessage : action.SendMessage;
        if (send.room_id !== this.snapshot.room) throw new Error("Wrong browser Room");
        return this.accept(await this.client.send_chat(send.text,
          "topic_id" in send && typeof send.topic_id === "string" ? send.topic_id : this.topic,
          "chat_id" in send && typeof send.chat_id === "string" ? send.chat_id : this.chat, send.metadata_json ?? undefined));
      }
      if ("StartTopicChat" in action || "StartTopicChatIntent" in action) {
        const start = "StartTopicChat" in action ? action.StartTopicChat : action.StartTopicChatIntent;
        await this.newChat(start.topic_id, "intent_key" in start && typeof start.intent_key === "string" ? start.intent_key : undefined);
      } else if ("CreateTopic" in action) {
        const topic = `topic-${crypto.randomUUID()}`;
        await this.publish("conversation_create", topic, { title: action.CreateTopic.title, description: null, external_topic: null, skill_binding: null });
        await this.newChat(topic);
      } else if ("OpenChat" in action) {
        this.topic = action.OpenChat.topic_id; this.chat = action.OpenChat.chat_id;
      } else if ("OpenTopic" in action) {
        this.topic = action.OpenTopic.topic_id;
        this.chat = this.state!.topics.find(topic => topic.topic_id === this.topic)?.active_chat_id ?? "";
      } else if ("RenameChat" in action) {
        await this.publish("namespaced", action.RenameChat.topic_id, action.RenameChat, "finitechat.chat.rename.v1");
      } else if ("SetChatArchived" in action) {
        await this.publish("namespaced", action.SetChatArchived.topic_id, action.SetChatArchived, "finitechat.chat.archive.v1");
      } else if (!("MarkRoomRead" in action || "SetTyping" in action || "StartRuntime" in action || "OpenRoom" in action || "RefreshDevices" in action)) {
        throw new Error("This action is not implemented in the browser spike yet.");
      }
      this.project();
      return this.state!;
    });
  }
  private project() {
    const s = this.snapshot;
    const topics = new Map<string, HostedChatTopic>();
    const messages: HostedChatMessage[] = [];
    const getTopic = (id: string, seq: number) => {
      let topic = topics.get(id);
      if (!topic) {
        topic = { room_id: s.room, topic_id: id, title: id === "home" ? "Home" : "Topic", last_message_preview: "",
          unread_count: 0, message_count: 0, created_seq: seq, updated_seq: seq, archived: false, active_chat_id: null, chats: [] };
        topics.set(id, topic);
      }
      return topic;
    };
    for (const event of [...s.events].sort((a, b) => a.seq - b.seq)) {
      const p = event.payload;
      if (!p) continue;
      const topic = getTopic(event.conversation_id ?? p.conversation_id ?? "home", event.seq);
      const type = event.kind.type;
      if (type === "conversation_create" || type === "conversation_update") {
        topic.title = p.title ?? topic.title;
      } else if (type === "conversation_segment_start" && p.segment_id) {
        topic.chats.forEach(chat => { chat.active = false; });
        topic.active_chat_id = p.segment_id;
        if (!topic.chats.some(chat => chat.chat_id === p.segment_id)) topic.chats.unshift({
          chat_id: p.segment_id, title: "New chat", last_message_preview: "", unread_count: 0, message_count: 0,
          started_seq: event.seq, updated_seq: event.seq, active: true, archived: false,
        });
      } else if (event.kind.name === "finitechat.chat.rename.v1" || event.kind.name === "finitechat.chat.archive.v1") {
        const chat = topics.get(p.topic_id ?? topic.topic_id)?.chats.find(chat => chat.chat_id === p.chat_id);
        if (chat) { if (p.title) chat.title = p.title; if (p.archived !== undefined) chat.archived = p.archived; }
      } else if ((type === "chat_message" || type === "chat_edit") && typeof p.text === "string") {
        const chatId = event.segment_id ?? p.segment_id ?? topic.active_chat_id;
        const chat = topic.chats.find(chat => chat.chat_id === chatId);
        const mine = event.sender.account_id === s.device.account_id;
        if (chat) {
          if (mine && chat.title === "New chat") chat.title = p.text.slice(0, 60);
          chat.last_message_preview = p.text.slice(0, 120); chat.updated_seq = event.seq; chat.message_count++;
        }
        topic.updated_seq = event.seq; topic.message_count++; topic.last_message_preview = p.text.slice(0, 120);
        messages.push({ room_id: s.room, seq: event.seq, message_id: event.id, conversation_id: topic.topic_id,
          chat_id: chatId, sender_account_id: event.sender.account_id, sender_device_id: event.sender.device_id,
          sender_display_name: mine ? "You" : p.sender_name || "Hermes", text: p.text, display_content: p.text,
          metadata_json: p.metadata ? JSON.stringify(p.metadata) : undefined, reply_to_message_id: p.reply_to_message_id,
          is_mine: mine, media: [], kind: p.kind ?? "message", status: p.status ?? "complete",
          final_delivery: p.status !== "running", edit_of_message_id: p.edit_of,
          timestamp_unix_seconds: event.timestamp, display_timestamp: new Date(event.timestamp * 1000).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" }),
        });
      }
    }
    this.state = { rev: ++this.revision, identity: s.device,
      rooms: [{ room_id: s.room, display_name: "Hermes", state: "Connected", status: "connected", user_status_text: "Connected",
        last_message_preview: messages.at(-1)?.text ?? "", unread_count: 0, can_load_older: false, is_agent_chat: true }],
      selected_room_id: s.room, topics: [...topics.values()], selected_topic_id: this.topic, selected_chat_id: this.chat,
      status: "connected", messages, profiles: [], devices: [], typing_members: [],
      hosted_agent_binding: { version: 1, project_id: "wasm-hermes", human_account_id: s.device.account_id,
        agent_account_id: this.agent, agent_npub: this.agentNpub, canonical_room_id: s.room, associated_room_ids: [] },
      flow: { notice_busy: false, scan_in_flight: false, scan_result: "" },
    };
    this.notify();
  }
}

// One Device per document, including React StrictMode remounts and sidebar navigation.
// Reload persistence is intentionally deferred and recorded in HOW_TO_DO_IT_FOR_REAL_LOG.
let session: Promise<BrowserFiniteChat> | undefined;
export function browserFiniteChatSession() {
  return session ??= (async () => { const client = new BrowserFiniteChat(); await client.connect(); return client; })()
    .catch(error => { session = undefined; throw error; });
}
